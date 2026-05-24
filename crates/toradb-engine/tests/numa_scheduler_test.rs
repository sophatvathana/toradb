use toradb_engine::{scheduler::SegmentScheduler, DagRunner};
use toradb_storage::NumaConfig;

#[test]
fn scheduler_merges_with_numa_config() {
    let scheduler = SegmentScheduler::new_with_numa(2, NumaConfig::default());
    let merged = scheduler.run_for_segments(2, true, |seg| {
        let mut c = toradb_core::CandidateSet::with_capacity(4);
        c.push(seg as u64 + 1, 1.0 - seg as f32 * 0.1);
        c
    });
    assert_eq!(merged.len(), 2);
}

#[test]
fn dag_open_applies_numa_env_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let dag = DagRunner::open(dir.path()).unwrap();
    assert!(dag.caches.numa.prefetch);
}
