//! Run with:
//!   cargo test -p toradb-index --release --test bm25_bench -- --nocapture --ignored
//! Not part of the normal test run (ignored by default).

use std::time::Instant;

use toradb_index::sparse::bm25::{Bm25Index, Bm25Snapshot};

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
}

fn build_corpus(num_docs: usize, vocab: usize, doc_len: usize) -> (Bm25Index, Vec<String>) {
    let words: Vec<String> = (0..vocab).map(|i| format!("w{i}")).collect();
    let mut rng = Rng(0x1234_5678_9abc_def0);
    let mut docs: Vec<(u64, String)> = Vec::with_capacity(num_docs);
    for d in 0..num_docs {
        let mut text = String::with_capacity(doc_len * 5);
        for _ in 0..doc_len {
            // Zipf-ish: square the random draw so small indices dominate.
            let r = rng.below(vocab);
            let idx = (r * r) / vocab;
            text.push_str(&words[idx.min(vocab - 1)]);
            text.push(' ');
        }
        docs.push((d as u64, text));
    }
    let snap = Bm25Snapshot::from_documents(docs.iter().map(|(id, t)| (*id, t.as_str())));
    (Bm25Index::from_snapshot(snap), words)
}

fn time_search(index: &Bm25Index, query: &str, k: usize, iters: usize) -> f64 {
    // warmup
    let _ = index.search(query, k);
    let start = Instant::now();
    for _ in 0..iters {
        let _ = index.search(query, k);
    }
    start.elapsed().as_secs_f64() * 1000.0 / iters as f64
}

#[test]
#[ignore]
fn bench_scale() {
    for &(num_docs, vocab, dl) in &[(500_000usize, 50_000usize, 120usize)] {
        let (index, words) = build_corpus(num_docs, vocab, dl);
        let k = 20;
        let iters = 30;
        let short = format!("{} {} {}", words[1], words[3], words[7]);
        // Natural-language-like long query: common head words + rare tail words.
        let long: String = (0..50)
            .map(|i| {
                if i % 3 == 0 {
                    words[i % 5].clone() // common
                } else {
                    words[(i * 911 + 13) % vocab].clone() // rare-ish
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let t_short = time_search(&index, &short, k, iters);
        let t_long = time_search(&index, &long, k, iters);
        println!(
            "\n=== scale {num_docs} docs / vocab {vocab} / dl {dl} ===\nshort: {t_short:8.3} ms   long(50): {t_long:8.3} ms  ({:.1}x)",
            t_long / t_short
        );
    }
}

#[test]
#[ignore]
fn bench_long_vs_short() {
    let num_docs = 100_000;
    let vocab = 20_000;
    let (index, words) = build_corpus(num_docs, vocab, 80);
    let k = 20;
    let iters = 50;

    // Short query: 3 common-ish words.
    let short = format!("{} {} {}", words[1], words[3], words[7]);
    // Medium query: 12 words.
    let medium: String = (0..12)
        .map(|i| words[i * 2 + 1].clone())
        .collect::<Vec<_>>()
        .join(" ");
    // Long query: 60 words (a pasted paragraph), mix of common + rare.
    let long: String = (0..60)
        .map(|i| words[(i * 37 + 1) % vocab].clone())
        .collect::<Vec<_>>()
        .join(" ");
    // Pathological: 60 of the single most common word repeated.
    let repeated: String = std::iter::repeat(words[0].as_str())
        .take(60)
        .collect::<Vec<_>>()
        .join(" ");

    let t_short = time_search(&index, &short, k, iters);
    let t_medium = time_search(&index, &medium, k, iters);
    let t_long = time_search(&index, &long, k, iters);
    let t_repeated = time_search(&index, &repeated, k, iters);

    println!("\n=== BM25 latency ({num_docs} docs, vocab {vocab}, doc_len 80) ===");
    println!("short   (3 terms):  {t_short:8.3} ms");
    println!("medium  (12 terms): {t_medium:8.3} ms");
    println!(
        "long    (60 terms): {t_long:8.3} ms  ({:.1}x short)",
        t_long / t_short
    );
    println!(
        "repeat  (60 dup):   {t_repeated:8.3} ms  ({:.1}x short)",
        t_repeated / t_short
    );
    println!();
}
