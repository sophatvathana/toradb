//! Run with:
//!   cargo test -p toradb-index --release --test sparse_bench -- --nocapture --ignored

use std::collections::HashMap;
use std::time::Instant;

use toradb_index::sparse::learned::{SparseProfile, SparseWeightedIndex};

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
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f32 {
        (self.next() % 1000) as f32 / 1000.0 + 0.001
    }
}

fn build(num_docs: usize, vocab: usize, expansion: usize) -> (SparseWeightedIndex, Vec<String>) {
    let words: Vec<String> = (0..vocab).map(|i| format!("t{i}")).collect();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut idx = SparseWeightedIndex::default();
    for d in 0..num_docs {
        let mut m = HashMap::new();
        for _ in 0..expansion {
            // Zipf-ish skew toward low ids (common tokens).
            let r = rng.below(vocab);
            let t = words[(r * r) / vocab].clone();
            m.insert(t, rng.unit());
        }
        idx.add_document(d as u64, &m);
    }
    (idx, words)
}

fn time(
    idx: &SparseWeightedIndex,
    q: &HashMap<String, f32>,
    profile: SparseProfile,
    iters: usize,
) -> f64 {
    let _ = idx.search_text(q, 20, profile);
    let start = Instant::now();
    for _ in 0..iters {
        let _ = idx.search_text(q, 20, profile);
    }
    start.elapsed().as_secs_f64() * 1000.0 / iters as f64
}

#[test]
#[ignore]
fn bench_splade_vs_seismic() {
    let num_docs = 200_000usize;
    let vocab = 30_000usize;
    let expansion = 120usize; // SPLADE-like per-doc token count
    let (idx, words) = build(num_docs, vocab, expansion);
    let iters = 30;

    let mut rng = Rng(0x1234_5678);
    let short: HashMap<String, f32> = (0..5)
        .map(|_| (words[rng.below(vocab)].clone(), rng.unit()))
        .collect();
    let long: HashMap<String, f32> = (0..150)
        .map(|_| (words[rng.below(vocab)].clone(), rng.unit()))
        .collect();

    println!("\n=== learned-sparse ({num_docs} docs, vocab {vocab}, expansion {expansion}) ===");
    for (name, q) in [("short(5)", &short), ("long(150)", &long)] {
        let s = time(&idx, q, SparseProfile::Splade, iters);
        let m = time(&idx, q, SparseProfile::Seismic, iters);
        println!(
            "{name:>10}: splade {s:8.3} ms   seismic {m:8.3} ms  ({:.2}x)",
            s / m.max(1e-6)
        );
    }
    println!();
}
