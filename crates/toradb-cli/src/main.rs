use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use toradb_api::ServeConfig;
use toradb_distributed::{ClusterClient, ClusterConfig, Worker};
use toradb_engine::persist;
use toradb_engine::{ingest_jsonl, ingest_parquet, DagRunner, IndexBuildPhase, IndexBuildState};
use toradb_index::dense::hnsw_index::HnswIndex;
use toradb_index::dense::turboquant;
use toradb_index::dense::turboquant_codec::{TqMode, TurboQuantSnapshot};
use toradb_index::dense::vector_codec::{self, VectorSnapshot};
use toradb_simd::dot_f32;

#[derive(Parser)]
#[command(name = "toradb-ingest", about = "ToraDB bulk ingest and index build")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Stream source data into Parquet segments (bulk mode), then finish indexes.
    Bulk(BulkArgs),
    /// Build segment BM25 sidecars after bulk load (no active bulk session required).
    Finish(FinishArgs),
    /// Resume or rerun index build (idempotent segment skip).
    Resume(FinishArgs),
    /// Run a distributed segment worker RPC server.
    Worker(WorkerArgs),
    /// Print cluster node health from `TORADB_CLUSTER_CONFIG` or `--config`.
    ClusterStatus(ClusterConfigArgs),
    /// Rebuild `segment_id_ranges` for legacy tables.
    RebuildIdRanges(FinishArgs),
    /// Run the TurboQuant validation sweep on a vector corpus.
    TqBench(TqBenchArgs),
    /// Platform dashboard and embedded API server commands.
    Platform {
        #[command(subcommand)]
        command: PlatformCommand,
    },
}

#[derive(Subcommand)]
enum PlatformCommand {
    /// Serve dashboard assets and API from one process.
    Serve(PlatformServeArgs),
}

#[derive(Parser)]
struct TqBenchArgs {
    /// Source kind: "db" (existing toradb sidecar) or "jsonl" (rows of `{"vector":[...]}`).
    #[arg(long, default_value = "db")]
    source: String,
    /// Database root (used when --source db).
    #[arg(long)]
    db: Option<PathBuf>,
    /// Table name (used when --source db).
    #[arg(long, default_value = "passages")]
    table: String,
    /// JSONL path (used when --source jsonl). Each row: `{"vector": [...]}`.
    #[arg(long)]
    path: Option<PathBuf>,
    /// Number of held-out queries to sample from the corpus.
    #[arg(long, default_value = "50")]
    queries: usize,
    /// Top-k.
    #[arg(long, default_value = "10")]
    k: usize,
    /// Cap on corpus size.
    #[arg(long, default_value = "50000")]
    limit: usize,
    /// HNSW EF_SEARCH values to sweep, comma-separated. Empty = use env or 64.
    #[arg(long, default_value = "")]
    ef_sweep: String,
    /// Re-rank oversampling factor (final k = k * factor candidates from HNSW).
    #[arg(long, default_value = "4")]
    rerank_factor: usize,
    /// Random seed for query selection and noise.
    #[arg(long, default_value = "12648430")] // 0xC0FFEE
    seed: u64,
}

#[derive(Parser)]
struct BulkArgs {
    #[arg(long)]
    db: PathBuf,
    #[arg(long, default_value = "passages")]
    table: String,
    #[arg(long, default_value = "parquet")]
    source: String,
    #[arg(long)]
    path: Option<PathBuf>,
    #[arg(long)]
    jsonl: Option<PathBuf>,
    #[arg(long, default_value = "200000")]
    batch_size: usize,
    #[arg(long, default_value = "0")]
    limit: u64,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    drop_table: bool,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    no_finish: bool,
}

#[derive(Parser)]
struct FinishArgs {
    #[arg(long)]
    db: PathBuf,
    #[arg(long, default_value = "passages")]
    table: String,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    compact: bool,
}

#[derive(Parser)]
struct WorkerArgs {
    #[arg(long)]
    db: PathBuf,
    #[arg(long, default_value = "127.0.0.1:9100")]
    addr: String,
}

#[derive(Parser)]
struct ClusterConfigArgs {
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Parser)]
struct PlatformServeArgs {
    #[arg(long)]
    db: PathBuf,
    #[arg(long, default_value = "127.0.0.1:8787")]
    addr: String,
    #[arg(long, default_value = "apps/platform/out")]
    static_dir: PathBuf,
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Bulk(args) => run_bulk(args),
        Commands::Finish(args) => run_index_op(args, false),
        Commands::Resume(args) => run_index_op(args, true),
        Commands::Worker(args) => {
            eprintln!("worker listening on {} db={}", args.addr, args.db.display());
            Worker::new(args.db).serve_blocking(&args.addr)
        }
        Commands::ClusterStatus(args) => run_cluster_status(args),
        Commands::RebuildIdRanges(args) => {
            persist::rebuild_segment_id_ranges(&args.db, &args.table)?;
            eprintln!("ok: rebuilt segment_id_ranges for {}", args.table);
            Ok(())
        }
        Commands::TqBench(args) => run_tq_bench(args),
        Commands::Platform { command } => run_platform(command),
    }
}

fn run_platform(cmd: PlatformCommand) -> Result<(), String> {
    match cmd {
        PlatformCommand::Serve(args) => {
            eprintln!(
                "platform serve on {} db={} static={}",
                args.addr,
                args.db.display(),
                args.static_dir.display()
            );
            toradb_api::serve_blocking(ServeConfig {
                db_path: args.db,
                listen_addr: args.addr,
                static_dir: args.static_dir,
            })
        }
    }
}

fn run_cluster_status(args: ClusterConfigArgs) -> Result<(), String> {
    let config = if let Some(path) = args.config {
        ClusterConfig::load(path)?
    } else {
        ClusterConfig::from_env()
            .ok_or("set TORADB_CLUSTER_CONFIG (YAML or JSON) or pass --config")?
    };
    let client = ClusterClient::new(config);
    for (id, ok) in client.health_all()? {
        eprintln!("{id}: {}", if ok { "healthy" } else { "down" });
    }
    Ok(())
}

fn run_index_op(args: FinishArgs, resume: bool) -> Result<(), String> {
    let label = if resume { "resume" } else { "finish" };
    eprintln!("{label}: building segment BM25 + indexes…");
    let t0 = Instant::now();
    let db = args.db.clone();
    let table = args.table.clone();
    let compact = args.compact;
    with_index_progress(&args.db, &args.table, || {
        let mut dag = DagRunner::open_with_reload(&db, false)?;
        dag.ensure_table(&table);
        dag.resume_index_build(&table, compact)
    })?;
    eprintln!("{label}: {:.1}s", t0.elapsed().as_secs_f64());
    Ok(())
}

fn with_index_progress(
    db: &Path,
    table: &str,
    work: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_watch = Arc::clone(&stop);
    let db_watch = db.to_path_buf();
    let table_watch = table.to_string();
    let watcher = thread::spawn(move || index_progress_loop(&db_watch, &table_watch, stop_watch));

    let result = work();
    stop.store(true, Ordering::Relaxed);
    watcher.join().ok();
    result
}

fn index_progress_loop(db: &Path, table: &str, stop: Arc<AtomicBool>) {
    let pb = ProgressBar::new(1);
    pb.set_style(
        ProgressStyle::with_template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏  "),
    );
    pb.enable_steady_tick(Duration::from_millis(120));

    while !stop.load(Ordering::Relaxed) {
        if let Some(status) = persist::read_table_index_build_status(db, table) {
            update_index_progress(&pb, &status);
        }
        thread::sleep(Duration::from_millis(200));
    }
    pb.finish_and_clear();
}

fn update_index_progress(pb: &ProgressBar, status: &toradb_engine::IndexBuildStatus) {
    let phase_label = status.phase.map(index_phase_label).unwrap_or("index build");

    match status.state {
        IndexBuildState::Ready => {
            pb.set_length(1);
            pb.set_position(1);
            pb.set_message("ready");
        }
        IndexBuildState::Failed => {
            let msg = status.message.clone().unwrap_or_else(|| "failed".into());
            pb.set_message(msg);
        }
        IndexBuildState::Building => {
            if status.phase == Some(IndexBuildPhase::SegmentBm25) && status.segments_total > 0 {
                pb.set_length(status.segments_total as u64);
                pb.set_position(status.segments_done as u64);
            } else {
                let total = status.segments_total.max(1) as u64;
                pb.set_length(total);
                pb.set_position(total);
            }
            pb.set_message(phase_label);
        }
    }
}

fn index_phase_label(phase: IndexBuildPhase) -> &'static str {
    match phase {
        IndexBuildPhase::SegmentBm25 => "segment BM25",
        IndexBuildPhase::MergeBm25 => "merge BM25",
        IndexBuildPhase::TableIndexes => "table indexes",
        IndexBuildPhase::ReloadTexts => "reload texts",
    }
}

fn run_bulk(args: BulkArgs) -> Result<(), String> {
    if args.drop_table {
        let table_dir = args.db.join(&args.table);
        if table_dir.exists() {
            std::fs::remove_dir_all(&table_dir).map_err(|e| e.to_string())?;
            eprintln!("dropped {}", table_dir.display());
        }
    }

    let mut dag = DagRunner::open_with_reload(&args.db, false)?;
    dag.ensure_table(&args.table);
    dag.begin_bulk_ingest(&args.table);

    let t0 = Instant::now();
    let total = match args.source.as_str() {
        "parquet" => {
            let path = args.path.ok_or("--path required for parquet")?;
            ingest_parquet(&mut dag, &args.table, &path, args.limit)?
        }
        "jsonl" => {
            let path = args
                .jsonl
                .or(args.path)
                .ok_or("--jsonl or --path required for jsonl")?;
            ingest_jsonl(&mut dag, &args.table, &path, args.batch_size, args.limit)?
        }
        other => return Err(format!("unsupported source {other}")),
    };

    eprintln!(
        "ingested {total} docs in {:.1}s ({:.0} docs/s)",
        t0.elapsed().as_secs_f64(),
        total as f64 / t0.elapsed().as_secs_f64().max(0.001)
    );

    if !args.no_finish {
        eprintln!("finish: building segment BM25 + indexes…");
        let t1 = Instant::now();
        let table = args.table.clone();
        with_index_progress(&args.db, &args.table, || {
            dag.finish_bulk_ingest(&table, false)
        })?;
        eprintln!("finish: {:.1}s", t1.elapsed().as_secs_f64());
    } else {
        eprintln!("skipped finish (--no-finish); run: toradb-ingest finish --db … --table …");
    }
    Ok(())
}

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
    fn gauss(&mut self) -> f32 {
        let u1 = ((self.next_u64() >> 11) as f64) / ((1u64 << 53) as f64);
        let u2 = ((self.next_u64() >> 11) as f64) / ((1u64 << 53) as f64);
        let u1 = u1.max(1e-300);
        ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
    }
}

fn load_corpus_from_db(
    db: &Path,
    table: &str,
    limit: usize,
) -> Result<Vec<(u64, Vec<f32>)>, String> {
    let mut pairs: Vec<(u64, Vec<f32>)> = Vec::new();
    if let Some(snap) = persist::load_table_vector_sidecar(db, table, None)? {
        let dim = snap.dim as usize;
        for (i, &id) in snap.ids.iter().enumerate() {
            let s = i * dim;
            pairs.push((id, snap.data[s..s + dim].to_vec()));
        }
    } else {
        let seg_dir = db.join(table).join("indexes");
        if !seg_dir.exists() {
            return Err(format!("indexes dir not found: {}", seg_dir.display()));
        }
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&seg_dir)
            .map_err(|e| e.to_string())?
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
            let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
            let snap = vector_codec::decode_snapshot(&bytes)?;
            let dim = snap.dim as usize;
            for (i, &id) in snap.ids.iter().enumerate() {
                let s = i * dim;
                pairs.push((id, snap.data[s..s + dim].to_vec()));
            }
            if pairs.len() >= limit {
                break;
            }
        }
    }
    if pairs.is_empty() {
        return Err(format!("no vectors found for table {table}"));
    }
    if pairs.len() > limit {
        pairs.truncate(limit);
    }
    Ok(pairs)
}

fn load_corpus_from_jsonl(path: &Path, limit: usize) -> Result<Vec<(u64, Vec<f32>)>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    let mut next_id = 0u64;
    for line in reader.lines() {
        if out.len() >= limit {
            break;
        }
        let line = line.map_err(|e| e.to_string())?;
        let row: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let v_field = row.get("vector").or_else(|| row.get("embedding"));
        let Some(arr) = v_field.and_then(|v| v.as_array()) else {
            continue;
        };
        let v: Vec<f32> = arr
            .iter()
            .filter_map(|x| x.as_f64().map(|x| x as f32))
            .collect();
        if v.is_empty() {
            continue;
        }
        let id = row.get("id").and_then(|v| v.as_u64()).unwrap_or_else(|| {
            let r = next_id;
            next_id += 1;
            r
        });
        out.push((id, v));
    }
    if out.is_empty() {
        return Err(format!("no vector rows found in {}", path.display()));
    }
    Ok(out)
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

fn run_tq_bench(args: TqBenchArgs) -> Result<(), String> {
    let load_t = Instant::now();
    let corpus = match args.source.as_str() {
        "db" => {
            let db = args.db.as_ref().ok_or("--db required for --source db")?;
            load_corpus_from_db(db, &args.table, args.limit)?
        }
        "jsonl" => {
            let p = args
                .path
                .as_ref()
                .ok_or("--path required for --source jsonl")?;
            let mut c = load_corpus_from_jsonl(p, args.limit)?;
            if c.len() > args.limit {
                c.truncate(args.limit);
            }
            c
        }
        other => return Err(format!("unsupported --source {other}")),
    };
    let dim = corpus[0].1.len();
    let f32_bytes = corpus.len() * dim * 4;
    eprintln!(
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
                *x += 0.1 * rng.gauss();
            }
            v
        })
        .collect();
    let truth_t = Instant::now();
    let truth: Vec<Vec<u64>> = queries
        .iter()
        .map(|q| brute_truth_topk(&corpus, q, args.k))
        .collect();
    eprintln!(
        "computed {} brute-force truth top-{} in {:.2}s",
        queries.len(),
        args.k,
        truth_t.elapsed().as_secs_f64(),
    );

    let ids: Vec<u64> = corpus.iter().map(|(id, _)| *id).collect();
    let vecs: Vec<Vec<f32>> = corpus.iter().map(|(_, v)| v.clone()).collect();
    let hnsw_t = Instant::now();
    let graph = HnswIndex::build(ids.clone(), vecs.clone()).ok_or("hnsw build failed")?;
    eprintln!(
        "built HNSW over {} vectors in {:.2}s",
        graph.len(),
        hnsw_t.elapsed().as_secs_f64(),
    );
    let full_snap = VectorSnapshot::from_pairs(dim as u32, &corpus).map_err(|e| e.to_string())?;

    let configs = [
        ("MSE 2b", TqMode::Mse, 2u8),
        ("MSE 3b", TqMode::Mse, 3),
        ("MSE 4b", TqMode::Mse, 4),
        ("IP  3b", TqMode::Ip, 3),
        ("IP  4b", TqMode::Ip, 4),
    ];

    let snaps: Vec<(&str, TurboQuantSnapshot, usize)> = configs
        .iter()
        .map(|(name, mode, bits)| {
            let snap = TurboQuantSnapshot::from_pairs(
                &corpus,
                *mode,
                *bits,
                0xABCD_1234_DEAD_BEEFu64,
                0xF00D_BABE_C0DE_BEEFu64,
            )
            .expect("encode");
            let bytes = toradb_index::dense::turboquant_codec::encode_snapshot(&snap)
                .expect("encode_snapshot");
            (*name, snap, bytes.len())
        })
        .collect();

    let ef_values: Vec<usize> = if !args.ef_sweep.is_empty() {
        args.ef_sweep
            .split(',')
            .map(|s| s.trim().parse::<usize>().expect("ef value"))
            .collect()
    } else if let Ok(v) = std::env::var("TORADB_HNSW_EF_SEARCH") {
        vec![v.parse().unwrap_or(64)]
    } else {
        vec![64]
    };

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

        for (name, snap, snap_bytes_len) in &snaps {
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
                    args.rerank_factor,
                );
                rerank_recall += recall(&truth[i], &got.ids);
            }
            let rerank_us = q_t.elapsed().as_secs_f64() * 1_000_000.0 / queries.len() as f64;
            rerank_recall /= queries.len() as f32;

            println!(
                "{:<10} {:>7.2} {:>9.2} {:>9.2}x {:>9.3} {:>8.0} {:>9.3} {:>8.0} {:>9.3} {:>8.0}",
                name,
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
    println!("rerank_factor={}", args.rerank_factor);
    Ok(())
}
