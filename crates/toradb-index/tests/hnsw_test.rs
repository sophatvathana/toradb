use toradb_index::dense::hnsw_index::HnswIndex;

fn unit(v: f32, dim: usize) -> Vec<f32> {
    let mut out = vec![0.0; dim];
    out[v as usize % dim] = 1.0;
    out
}

#[test]
fn hnsw_finds_nearest_of_many_vectors() {
    let dim = 8;
    let mut ids = Vec::new();
    let mut vectors = Vec::new();
    for i in 0..40u64 {
        ids.push(i);
        vectors.push(unit(i as f32, dim));
    }
    let index = HnswIndex::build(ids, vectors).expect("hnsw");
    let q = unit(39.0, dim);
    let hits = index.search(&q, 3);
    assert!(!hits.ids.is_empty());
    assert_eq!(hits.ids[0], 39);
}
