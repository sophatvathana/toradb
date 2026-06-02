use std::collections::HashMap;

use toradb_engine::{run_table_search, DagRunner, TableSearchOptions};
use toradb_index::IngestDoc;

fn fake_encode(text: &str) -> HashMap<String, f32> {
    let mut m = HashMap::new();
    for tok in text.split_whitespace() {
        let t = tok.to_lowercase();
        let w = t.len() as f32;
        m.entry(t).and_modify(|e| *e += w).or_insert(w);
    }
    m
}

fn doc(text: &str) -> IngestDoc {
    IngestDoc {
        text: text.into(),
        metadata: HashMap::new(),
        vector: None,
        sparse: Some(fake_encode(text)),
    }
}

fn opts(table: &str, query: &str, query_sparse: Option<HashMap<String, f32>>) -> TableSearchOptions {
    TableSearchOptions {
        table: table.into(),
        query: query.into(),
        top_k: Some(10),
        offset: None,
        strategy: Some("splade".into()),
        explain: false,
        graph_expand: None,
        depth: None,
        query_vector: None,
        facets: Vec::new(),
        facet_top_n: None,
        query_sparse,
        bm25_params: None,
        field_boosts: std::collections::HashMap::new(),
        decay: None,
    }
}

#[test]
fn splade_uses_learned_weights() {
    let dir = std::env::temp_dir().join("toradb_learned_sparse_splade");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");

    dag.add_documents(
        "docs",
        vec![
            doc("tesla alternating"),
            doc("tesla ac"),
        ],
    )
    .expect("add");

    let q = fake_encode("tesla alternating");
    let res = run_table_search(&mut dag, opts("docs", "tesla alternating", Some(q))).unwrap();
    assert!(!res.ids.is_empty());
    assert_eq!(res.ids[0], 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn splade_falls_back_to_bm25_without_query_sparse() {
    let dir = std::env::temp_dir().join("toradb_learned_sparse_fallback");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![doc("Nikola Tesla alternating current motor"), doc("Marie Curie radioactivity")],
    )
    .expect("add");

    let res = run_table_search(&mut dag, opts("docs", "Nikola Tesla motor", None)).unwrap();
    assert!(!res.ids.is_empty());
    assert_eq!(res.ids[0], 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn learned_sparse_survives_reload() {
    let dir = std::env::temp_dir().join("toradb_learned_sparse_reload");
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents("docs", vec![doc("tesla alternating"), doc("tesla ac")])
            .expect("add");
        let q = fake_encode("tesla alternating");
        let _ = run_table_search(&mut dag, opts("docs", "tesla alternating", Some(q))).unwrap();
    }
    let mut dag2 = DagRunner::open(&dir).expect("reopen");
    let q = fake_encode("tesla alternating");
    let res = run_table_search(&mut dag2, opts("docs", "tesla alternating", Some(q))).unwrap();
    assert!(!res.ids.is_empty());
    assert_eq!(res.ids[0], 0);

    let _ = std::fs::remove_dir_all(&dir);
}
