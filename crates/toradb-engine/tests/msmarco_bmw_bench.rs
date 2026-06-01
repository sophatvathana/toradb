//! Run with:
//!   cargo test -p toradb-engine --release --test msmarco_bmw_bench -- --nocapture --ignored


use std::path::PathBuf;
use std::time::Instant;

use toradb_core::{Batch, CandidateSet, ExecCtx};
use toradb_engine::persist;
use toradb_engine::DagRunner;

fn msmarco_root() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/msmarco_1m");
    if root.join("passages/manifest.json").is_file() {
        Some(root)
    } else {
        None
    }
}

#[test]
#[ignore]
fn bench_msmarco_long_queries() {
    let Some(base) = msmarco_root() else {
        eprintln!("skip: data/msmarco_1m not found");
        return;
    };
    let table = "passages";
    persist::rebuild_segment_id_ranges(&base, table).expect("rebuild id ranges");
    let mut dag = DagRunner::open_with_reload(&base, false).expect("open");

    let ctx = ExecCtx::new(1000, 100, 100);
    let queries: &[(&str, &str)] = &[
        ("short(3)", "treatment for diabetes"),
        (
            "long(NL)",
            "what is the recommended medical treatment and lifestyle changes for managing \
             type 2 diabetes in older adults including diet exercise and medication options",
        ),
        (
            "repetitive",
            "do it do it like i wanna do it do it you gon know me like you ain't \
             never known me do it do it before",
        ),
        (
            "very-long",
            "the quick brown fox jumps over the lazy dog and then the dog runs away from \
             the fox while the cat watches the whole scene from the top of the fence near \
             the old red barn where the farmer keeps his tools and the chickens roam free \
             every single morning before the sun comes up over the green rolling hills",
        ),
    ];

    let run_once = |dag: &mut DagRunner, q: &str| -> (usize, std::time::Duration) {
        let mut batch = Batch::new();
        batch.table = table.into();
        batch.query = q.into();
        batch.tier1_enable_sparse = true;
        batch.candidates = CandidateSet::default();
        let t = Instant::now();
        dag.run(&mut batch, &ctx);
        (batch.candidates.len(), t.elapsed())
    };

    eprintln!("\n=== MSMARCO 8.84M passages — real disk TBM3 BMW path ===");
    for (label, q) in queries {
        // Warm the cache, then take the best of 3 warm runs (steady state).
        let (_n0, cold) = run_once(&mut dag, q);
        let mut best = std::time::Duration::from_secs(3600);
        let mut hits = 0;
        for _ in 0..3 {
            let (n, d) = run_once(&mut dag, q);
            hits = n;
            if d < best {
                best = d;
            }
        }
        let toks = q.split_whitespace().count();
        eprintln!(
            "{label:12} {toks:3} toks | cold {:7.1} ms | warm {:7.1} ms | {hits} hits",
            cold.as_secs_f64() * 1000.0,
            best.as_secs_f64() * 1000.0,
        );
    }
    eprintln!();
}
