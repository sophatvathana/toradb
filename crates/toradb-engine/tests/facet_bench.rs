//! Run with:
//!   cargo test -p toradb-engine --release --test facet_bench -- --nocapture --ignored

use std::collections::HashSet;
use std::time::Instant;

use toradb_engine::{count_facets, DagRunner};
use toradb_index::IngestDoc;

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

fn build_table(dag: &mut DagRunner, num_docs: usize, cardinality: usize) {
    let mut rng = Rng(0x51ed_5eed_1234_abcd);
    // Ingest in chunks to keep memory bounded.
    let chunk = 5_000;
    let mut buf = Vec::with_capacity(chunk);
    for d in 0..num_docs {
        let cat = format!("cat{}", rng.below(cardinality));
        buf.push(IngestDoc {
            text: format!("Nikola Tesla document {d}"),
            metadata: [("category".into(), cat)].into(),
            vector: None,
        });
        if buf.len() == chunk {
            dag.add_documents("docs", std::mem::take(&mut buf)).unwrap();
        }
    }
    if !buf.is_empty() {
        dag.add_documents("docs", buf).unwrap();
    }
}

fn time_facets(dag: &mut DagRunner, candidates: &HashSet<u64>, iters: usize) -> f64 {
    let fields = vec!["category".to_string()];
    let _ = count_facets(dag, "docs", &fields, candidates, 20).unwrap();
    let start = Instant::now();
    for _ in 0..iters {
        let _ = count_facets(dag, "docs", &fields, candidates, 20).unwrap();
    }
    start.elapsed().as_secs_f64() * 1000.0 / iters as f64
}

#[test]
#[ignore]
fn bench_facets_by_candidate_set_size() {
    let num_docs = 200_000usize;
    let cardinality = 50usize;
    let dir = std::env::temp_dir().join("toradb_facet_bench");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    build_table(&mut dag, num_docs, cardinality);

    let iters = 20;
    println!("\n=== facet count ({num_docs} docs, cardinality {cardinality}) ===");
    for &frac in &[0.01_f64, 0.1, 0.5, 1.0] {
        let n = ((num_docs as f64) * frac) as u64;
        let candidates: HashSet<u64> = (0..n).collect();
        let ms = time_facets(&mut dag, &candidates, iters);
        println!("candidates={n:>8} ({:>4.0}%): {ms:8.3} ms", frac * 100.0);
    }
    println!();

    let _ = std::fs::remove_dir_all(&dir);
}
