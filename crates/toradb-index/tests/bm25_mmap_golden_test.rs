use toradb_index::sparse::bm25::{Bm25Index, Bm25Snapshot};
use toradb_index::sparse::bm25_tbm3::{encode_tbm3, snapshot_from_tbm3, Bm25Tbm3View};

#[test]
fn tbm3_mmap_search_returns_hits() {
    let snap = Bm25Snapshot::from_documents([(0u64, "alpha beta gamma tesla motor")]);
    let golden = snap.search("tesla motor", 5);
    assert!(!golden.is_empty());

    let bytes = encode_tbm3(&snap);
    let roundtrip = snapshot_from_tbm3(&bytes).expect("decode");
    assert!(!roundtrip.search("tesla motor", 5).is_empty());

    let view = Bm25Tbm3View::open(&bytes).expect("tbm3");
    assert!(!view.search("tesla motor", 5).is_empty());
}

#[test]
fn interned_merge_matches_tree_merge() {
    let a = Bm25Snapshot::from_documents([(0u64, "alpha beta")]);
    let b = Bm25Snapshot::from_documents([(1u64, "gamma delta")]);
    let tree = Bm25Snapshot::merge_snapshots_tree(vec![a.clone(), b.clone()]).unwrap();
    let interned = Bm25Snapshot::merge_snapshots_interned(vec![a, b]).unwrap();
    let t1 = Bm25Index::from_snapshot(tree);
    let t2 = Bm25Index::from_snapshot(interned);
    assert_eq!(t1.search("alpha", 5).ids, t2.search("alpha", 5).ids);
    assert_eq!(t1.search("gamma", 5).ids, t2.search("gamma", 5).ids);
}
