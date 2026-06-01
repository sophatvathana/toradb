//! Latency benchmark for the on-disk TBM3 BM25 path (Block-Max WAND). Run with:
//!   cargo test -p toradb-index --release --test tbm3_bench -- --nocapture --ignored

use std::time::Instant;

use toradb_index::sparse::bm25::Bm25Snapshot;
use toradb_index::sparse::bm25_tbm3::{encode_tbm3, Bm25Tbm3View};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[test]
#[ignore]
fn bench_tbm3_long_query() {
    let num_docs = 1_000_000usize;
    let vocab = 60_000usize;
    let doc_len = 100usize;
    let words: Vec<String> = (0..vocab).map(|i| format!("w{i}")).collect();
    let mut rng = Rng(0xdead_beef_1234_5678);

    let mut docs: Vec<(u64, String)> = Vec::with_capacity(num_docs);
    for d in 0..num_docs {
        let mut text = String::with_capacity(doc_len * 6);
        for _ in 0..doc_len {
            let r = (rng.next() % vocab as u64) as usize;
            let idx = (r * r) / vocab;
            text.push_str(&words[idx.min(vocab - 1)]);
            text.push(' ');
        }
        docs.push((d as u64, text));
    }
    let build = Instant::now();
    let snap = Bm25Snapshot::from_documents(docs.iter().map(|(i, t)| (*i, t.as_str())));
    let bytes = encode_tbm3(&snap);
    let view = Bm25Tbm3View::open(&bytes).unwrap();
    println!(
        "\n=== TBM3 {num_docs} docs / vocab {vocab} / dl {doc_len} (built+encoded in {:.1}s, {} MB) ===",
        build.elapsed().as_secs_f64(),
        bytes.len() / 1_000_000
    );

    let k = 20;
    let iters = 20;
    let time = |q: &str| -> f64 {
        let _ = view.search(q, k);
        let s = Instant::now();
        for _ in 0..iters {
            let _ = view.search(q, k);
        }
        s.elapsed().as_secs_f64() * 1000.0 / iters as f64
    };

    let short = format!("{} {} {}", words[2], words[5], words[11]);
    let long: String = (0..40)
        .map(|i| {
            if i % 4 == 0 {
                words[i % 6].clone() // very common
            } else {
                words[(i * 877 + 7) % vocab].clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let repetitive = format!(
        "{w0} {w1} {w1} {w0} {w1} {w0} {w0} {w1} {w2} {w1} {w0} {w1}",
        w0 = words[0], w1 = words[1], w2 = words[2]
    );

    let t_short = time(&short);
    let t_long = time(&long);
    let t_rep = time(&repetitive);
    println!("short(3): {t_short:8.2} ms    long(40): {t_long:8.2} ms  ({:.1}x)", t_long / t_short);
    println!("repetitive(12 toks, 3 uniq): {t_rep:8.2} ms\n");
}
