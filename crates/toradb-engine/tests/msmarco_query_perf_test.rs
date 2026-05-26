//! Optional integration test when `data/msmarco_1m` is present locally.

use std::path::PathBuf;
use std::time::Instant;

use toradb_core::{Batch, ExecCtx};
use toradb_engine::DagRunner;
use toradb_engine::persist;
use toradb_storage::columnar::IndexMode;
use toradb_storage::columnar::TableManifestFile;

fn msmarco_root() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/msmarco_1m");
    let manifest = root.join("passages/manifest.json");
    if manifest.is_file() {
        Some(root)
    } else {
        None
    }
}

#[test]
fn msmarco_segment_only_search_warm_cache() {
    let Some(base) = msmarco_root() else {
        eprintln!("skip msmarco_query_perf_test: data/msmarco_1m not found");
        return;
    };
    let table = "passages";
    persist::rebuild_segment_id_ranges(&base, table).expect("rebuild id ranges");
    let manifest =
        TableManifestFile::load(&TableManifestFile::path_for_table(&base, table)).expect("manifest");
    assert_eq!(manifest.index_mode, IndexMode::SegmentOnly);
    assert!(!manifest.segment_id_ranges.is_empty());

    let mut dag = DagRunner::open_with_reload(&base, false).expect("open");
    let mut batch = Batch::new();
    batch.table = table.into();
    batch.query = "what is the treatment for diabetes".into();
    batch.tier1_enable_sparse = true;
    let ctx = ExecCtx::new(200, 100, 100);

    let t0 = Instant::now();
    dag.run(&mut batch, &ctx);
    let first = t0.elapsed();
    assert!(!batch.candidates.is_empty(), "expected hits");

    batch.candidates = toradb_core::CandidateSet::default();
    let t1 = Instant::now();
    dag.run(&mut batch, &ctx);
    let second = t1.elapsed();
    assert!(!batch.candidates.is_empty());

    eprintln!("msmarco search first {:?} second {:?}", first, second);
    assert!(
        second <= first.saturating_mul(3),
        "warm search should not be much slower than first"
    );

    let hit_ids: Vec<u64> = batch
        .candidates
        .ids
        .iter()
        .copied()
        .take(100)
        .collect();
    if !hit_ids.is_empty() {
        let t_fetch = std::time::Instant::now();
        let docs = dag.fetch_documents(table, &hit_ids).expect("fetch");
        let fetch_elapsed = t_fetch.elapsed();
        eprintln!(
            "msmarco fetch {} ids -> {} docs in {:?}",
            hit_ids.len(),
            docs.len(),
            fetch_elapsed
        );
        assert_eq!(docs.len(), hit_ids.len());
        let t_fetch2 = std::time::Instant::now();
        let docs2 = dag.fetch_documents(table, &hit_ids).expect("fetch2");
        eprintln!("msmarco fetch repeat {:?} ({} docs)", t_fetch2.elapsed(), docs2.len());
    }
}
