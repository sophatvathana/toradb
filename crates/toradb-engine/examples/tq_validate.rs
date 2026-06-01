//! TurboQuant validation
//!
//! Loads an existing toradb table (default: `data/msmarco_1m/passages`), reads
//! the f32 vectors out of its sidecars, builds TurboQuant snapshots at several
//! configurations, and reports for each:
//!   * codec size in bytes (per-vector and total)
//!   * encode throughput
//!   * recall@10 vs full-precision ground truth
//!   * ADC query latency (single-thread brute force)
//!
//! Usage:
//!   cargo run --release -p toradb-engine --example tq_validate -- \
//!       --db data/msmarco_1m/passages --table passages \
//!       --queries 64 --k 10 --limit 50000
//!
//! All flags are optional; defaults are tuned for a quick first look.

use std::path::PathBuf;
use std::time::Instant;

use toradb_engine::persist;
use toradb_index::dense::hnsw_index::HnswIndex;
use toradb_index::dense::turboquant;
use toradb_index::dense::turboquant_codec::{TqMode, TurboQuantSnapshot};
use toradb_index::dense::vector_codec::{self, VectorSnapshot};
use toradb_simd::dot_f32;

#[derive(Clone)]
struct Args {
    db: PathBuf,
    table: String,
    queries: usize,
    k: usize,
    limit: usize,
    seed: u64,
    synthetic_dim: Option<usize>,
    synthetic_n: usize,
    ef_sweep: Vec<usize>,
}

fn parse_args() -> Args {
    let mut db = PathBuf::from("examples/_demo_db");
    let mut table = String::from("ann_corpus");
    let mut queries = 64usize;
    let mut k = 10usize;
    let mut limit = 50_000usize;
    let mut seed = 0xC0FFEEu64;
    let mut synthetic_dim: Option<usize> = None;
    let mut synthetic_n: usize = 50_000;
    let mut ef_sweep: Vec<usize> = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--db" => db = PathBuf::from(it.next().expect("--db needs a value")),
            "--table" => table = it.next().expect("--table needs a value"),
            "--queries" => queries = it.next().unwrap().parse().unwrap(),
            "--k" => k = it.next().unwrap().parse().unwrap(),
            "--limit" => limit = it.next().unwrap().parse().unwrap(),
            "--seed" => seed = it.next().unwrap().parse().unwrap(),
            "--synthetic-dim" => synthetic_dim = Some(it.next().unwrap().parse().unwrap()),
            "--synthetic-n" => synthetic_n = it.next().unwrap().parse().unwrap(),
            "--ef-sweep" => {
                ef_sweep = it
                    .next()
                    .unwrap()
                    .split(',')
                    .map(|s| s.trim().parse::<usize>().expect("ef value"))
                    .collect();
            }
            "-h" | "--help" => {
                eprintln!(
                    "tq_validate [--db PATH] [--table NAME] [--queries N] [--k N] \
                     [--limit N] [--seed N] [--synthetic-dim D --synthetic-n N]"
                );
                std::process::exit(0);
            }
            other => panic!("unknown arg {other}"),
        }
    }
    Args {
        db,
        table,
        queries,
        k,
        limit,
        seed,
        synthetic_dim,
        synthetic_n,
        ef_sweep,
    }
}

/// Deterministic xorshift for reproducible held-out query selection.
struct XorShift(u64);
impl XorShift {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn next_in(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Generate `n` unit-norm Gaussian vectors of dimension `d` for synthetic
/// benchmarking when no real corpus is available.
fn synth_corpus(d: usize, n: usize, seed: u64) -> Vec<(u64, Vec<f32>)> {
    let mut rng = XorShift(seed | 1);
    let mut out = Vec::with_capacity(n);
    for id in 0..n {
        let mut v = vec![0f32; d];
        let mut i = 0;
        while i + 1 < d {
            // Box–Muller pair
            let u1 = ((rng.next_u64() >> 11) as f64) / ((1u64 << 53) as f64);
            let u2 = ((rng.next_u64() >> 11) as f64) / ((1u64 << 53) as f64);
            let u1 = u1.max(1e-300);
            let r = (-2.0 * u1.ln()).sqrt();
            let theta = 2.0 * std::f64::consts::PI * u2;
            v[i] = (r * theta.cos()) as f32;
            v[i + 1] = (r * theta.sin()) as f32;
            i += 2;
        }
        if i < d {
            let u1 = ((rng.next_u64() >> 11) as f64) / ((1u64 << 53) as f64);
            let u2 = ((rng.next_u64() >> 11) as f64) / ((1u64 << 53) as f64);
            let u1 = u1.max(1e-300);
            v[i] = ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32;
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
        for x in &mut v {
            *x /= norm;
        }
        out.push((id as u64, v));
    }
    out
}

fn load_vectors(args: &Args) -> Vec<(u64, Vec<f32>)> {
    // Prefer the merged table-level f32 sidecar; fall back to scanning per-segment.
    let mut pairs: Vec<(u64, Vec<f32>)> = Vec::new();
    if let Some(snap) =
        persist::load_table_vector_sidecar(&args.db, &args.table, None).expect("read table sidecar")
    {
        let dim = snap.dim as usize;
        for (i, &id) in snap.ids.iter().enumerate() {
            let s = i * dim;
            pairs.push((id, snap.data[s..s + dim].to_vec()));
        }
    } else {
        let seg_dir = args.db.join(&args.table).join("indexes");
        if !seg_dir.exists() {
            panic!("indexes dir not found: {}", seg_dir.display());
        }
        let mut paths: Vec<_> = std::fs::read_dir(&seg_dir)
            .expect("read indexes dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("seg_") && n.ends_with(".vectors.bin"))
                    .unwrap_or(false)
            })
            .collect();
        paths.sort();
        for p in paths {
            let bytes = std::fs::read(&p).expect("read sidecar");
            let snap = vector_codec::decode_snapshot(&bytes).expect("decode sidecar");
            let dim = snap.dim as usize;
            for (i, &id) in snap.ids.iter().enumerate() {
                let s = i * dim;
                pairs.push((id, snap.data[s..s + dim].to_vec()));
            }
            if pairs.len() >= args.limit {
                break;
            }
        }
    }
    if pairs.is_empty() {
        panic!(
            "no vector sidecars found for table '{}' under {}",
            args.table,
            args.db.display()
        );
    }
    if pairs.len() > args.limit {
        pairs.truncate(args.limit);
    }
    pairs
}

fn brute_truth_topk(corpus: &[(u64, Vec<f32>)], query: &[f32], k: usize) -> Vec<u64> {
    let mut scored: Vec<(u64, f32)> = corpus
        .iter()
        .map(|(id, v)| (*id, dot_f32(query, v)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored.into_iter().map(|(id, _)| id).collect()
}

fn adc_topk(snap: &TurboQuantSnapshot, query: &[f32], k: usize) -> Vec<u64> {
    let qrot = snap.rotate_query(query);
    let mut scored: Vec<(u64, f32)> = (0..snap.len())
        .map(|i| (snap.ids[i], snap.adc_dot(&qrot, i)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored.into_iter().map(|(id, _)| id).collect()
}

fn recall(truth: &[u64], got: &[u64]) -> f32 {
    if truth.is_empty() {
        return 1.0;
    }
    let hits = got.iter().filter(|id| truth.contains(id)).count();
    hits as f32 / truth.len() as f32
}

struct Config {
    name: &'static str,
    mode: TqMode,
    bits: u8,
}

fn main() {
    let args = parse_args();
    println!("# TurboQuant validation");
    println!(
        "db={} table={} queries={} k={} limit={}",
        args.db.display(),
        args.table,
        args.queries,
        args.k,
        args.limit
    );

    let load_t = Instant::now();
    let corpus = if let Some(d) = args.synthetic_dim {
        synth_corpus(d, args.synthetic_n, args.seed)
    } else {
        load_vectors(&args)
    };
    let dim = corpus[0].1.len();
    let f32_bytes = corpus.len() * dim * 4;
    println!(
        "loaded {} vectors (dim={}) in {:.2}s, raw f32 size = {:.2} MiB",
        corpus.len(),
        dim,
        load_t.elapsed().as_secs_f64(),
        f32_bytes as f64 / (1024.0 * 1024.0),
    );

    let mut rng = XorShift(args.seed | 1);
    let mut q_indices: Vec<usize> = (0..args.queries.min(corpus.len()))
        .map(|_| rng.next_in(corpus.len()))
        .collect();
    q_indices.sort();
    q_indices.dedup();
    let queries: Vec<Vec<f32>> = q_indices
        .iter()
        .map(|&i| {
            let mut v = corpus[i].1.clone();
            for x in &mut v {
                let u1 = ((rng.next_u64() >> 11) as f64) / ((1u64 << 53) as f64);
                let u2 = ((rng.next_u64() >> 11) as f64) / ((1u64 << 53) as f64);
                let u1 = u1.max(1e-300);
                let g = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                *x += 0.1 * g as f32;
            }
            v
        })
        .collect();
    let mut truth: Vec<Vec<u64>> = Vec::with_capacity(queries.len());
    let truth_t = Instant::now();
    for q in &queries {
        truth.push(brute_truth_topk(&corpus, q, args.k));
    }
    println!(
        "computed {} brute-force truth top-{} in {:.2}s",
        queries.len(),
        args.k,
        truth_t.elapsed().as_secs_f64(),
    );

    // Build HNSW once and a VectorSnapshot for full-precision re-rank.
    let ids: Vec<u64> = corpus.iter().map(|(id, _)| *id).collect();
    let vecs: Vec<Vec<f32>> = corpus.iter().map(|(_, v)| v.clone()).collect();
    let hnsw_t = Instant::now();
    let graph = HnswIndex::build(ids.clone(), vecs.clone()).expect("hnsw build");
    println!(
        "built HNSW over {} vectors in {:.2}s",
        graph.len(),
        hnsw_t.elapsed().as_secs_f64(),
    );
    let full_snap = VectorSnapshot::from_pairs(dim as u32, &corpus).expect("full snap");

    let configs = [
        Config {
            name: "MSE 2b",
            mode: TqMode::Mse,
            bits: 2,
        },
        Config {
            name: "MSE 3b",
            mode: TqMode::Mse,
            bits: 3,
        },
        Config {
            name: "MSE 4b",
            mode: TqMode::Mse,
            bits: 4,
        },
        Config {
            name: "IP  3b",
            mode: TqMode::Ip,
            bits: 3,
        },
        Config {
            name: "IP  4b",
            mode: TqMode::Ip,
            bits: 4,
        },
    ];

    let rerank_factor = 4usize;

    let ef_values: Vec<usize> = if !args.ef_sweep.is_empty() {
        args.ef_sweep.clone()
    } else if let Ok(v) = std::env::var("TORADB_HNSW_EF_SEARCH") {
        vec![v.parse().unwrap_or(64)]
    } else {
        vec![64]
    };

    let snaps: Vec<(&Config, TurboQuantSnapshot, usize)> = configs
        .iter()
        .map(|cfg| {
            let snap = TurboQuantSnapshot::from_pairs(
                &corpus,
                cfg.mode,
                cfg.bits,
                0xABCD_1234_DEAD_BEEFu64,
                0xF00D_BABE_C0DE_BEEFu64,
            )
            .expect("encode");
            let bytes = toradb_index::dense::turboquant_codec::encode_snapshot(&snap)
                .expect("encode_snapshot");
            (cfg, snap, bytes.len())
        })
        .collect();

    for &ef in &ef_values {
        std::env::set_var("TORADB_HNSW_EF_SEARCH", ef.to_string());
        println!();
        println!("=== EF_SEARCH={ef} ===");
        println!(
            "{:<10} {:>7} {:>9} {:>10} {:>9} {:>8} {:>9} {:>8} {:>9} {:>8}",
            "codec",
            "bits/d",
            "size MiB",
            "compress",
            "brute_R",
            "brute_us",
            "hnsw_R",
            "hnsw_us",
            "rerank_R",
            "rerank_us",
        );
        println!("{}", "-".repeat(96));

        for (cfg, snap, snap_bytes_len) in &snaps {
            let size_mib = *snap_bytes_len as f64 / (1024.0 * 1024.0);
            let bits_per_dim = (*snap_bytes_len * 8) as f64 / (corpus.len() as f64 * dim as f64);

            let q_t = Instant::now();
            let mut brute_recall = 0.0f32;
            for (i, q) in queries.iter().enumerate() {
                let got = adc_topk(snap, q, args.k);
                brute_recall += recall(&truth[i], &got);
            }
            let brute_us = q_t.elapsed().as_secs_f64() * 1_000_000.0 / queries.len() as f64;
            brute_recall /= queries.len() as f32;

            let q_t = Instant::now();
            let mut hnsw_recall = 0.0f32;
            for (i, q) in queries.iter().enumerate() {
                let got = turboquant::hnsw_adc_search(&graph, snap, None, q, args.k, 1);
                hnsw_recall += recall(&truth[i], &got.ids);
            }
            let hnsw_us = q_t.elapsed().as_secs_f64() * 1_000_000.0 / queries.len() as f64;
            hnsw_recall /= queries.len() as f32;

            let q_t = Instant::now();
            let mut rerank_recall = 0.0f32;
            for (i, q) in queries.iter().enumerate() {
                let got = turboquant::hnsw_adc_search(
                    &graph,
                    snap,
                    Some(&full_snap),
                    q,
                    args.k,
                    rerank_factor,
                );
                rerank_recall += recall(&truth[i], &got.ids);
            }
            let rerank_us = q_t.elapsed().as_secs_f64() * 1_000_000.0 / queries.len() as f64;
            rerank_recall /= queries.len() as f32;

            println!(
                "{:<10} {:>7.2} {:>9.2} {:>9.2}x {:>9.3} {:>8.0} {:>9.3} {:>8.0} {:>9.3} {:>8.0}",
                cfg.name,
                bits_per_dim,
                size_mib,
                f32_bytes as f64 / *snap_bytes_len as f64,
                brute_recall,
                brute_us,
                hnsw_recall,
                hnsw_us,
                rerank_recall,
                rerank_us,
            );
        }

        let q_t = Instant::now();
        let mut hnsw_hit_sum = 0.0f32;
        for (i, q) in queries.iter().enumerate() {
            let got = graph.search(q, args.k);
            hnsw_hit_sum += recall(&truth[i], &got.ids);
        }
        let hnsw_f32_us = q_t.elapsed().as_secs_f64() * 1_000_000.0 / queries.len() as f64;
        println!(
            "{:<10}                                                                  recall={:.3}  us/query={:.0}",
            "HNSW f32",
            hnsw_hit_sum / queries.len() as f32,
            hnsw_f32_us,
        );
    }

    let q_t = Instant::now();
    let mut hit_sum = 0.0f32;
    for (i, q) in queries.iter().enumerate() {
        let got = brute_truth_topk(&corpus, q, args.k);
        hit_sum += recall(&truth[i], &got);
    }
    let brute_f32_us = q_t.elapsed().as_secs_f64() * 1_000_000.0 / queries.len() as f64;
    println!();
    println!(
        "ceiling brute f32:  recall={:.3}  us/query={:.0}  size={:.2} MiB (1.00x)",
        hit_sum / queries.len() as f32,
        brute_f32_us,
        f32_bytes as f64 / (1024.0 * 1024.0),
    );
    println!(
        "rerank_factor={} (HNSW returns k*factor candidates, then re-ranks with full f32)",
        rerank_factor,
    );
}
