use std::collections::HashMap;

use toradb_engine::{run_table_search, DagRunner, TableSearchOptions};
use toradb_index::IngestDoc;

fn doc(text: &str) -> IngestDoc {
    IngestDoc {
        text: text.into(),
        metadata: HashMap::new(),
        vector: None,
        sparse: None,
    }
}

fn opts(table: &str, query: &str, highlight: bool, snippet_len: u32) -> TableSearchOptions {
    TableSearchOptions {
        table: table.into(),
        query: query.into(),
        top_k: Some(10),
        offset: None,
        strategy: Some("sparse".into()),
        explain: false,
        graph_expand: None,
        depth: None,
        query_vector: None,
        facets: Vec::new(),
        facet_top_n: None,
        query_sparse: None,
        bm25_params: None,
        field_boosts: HashMap::new(),
        decay: None,
        highlight,
        snippet_len,
    }
}

#[test]
fn highlight_returns_marked_snippets() {
    let dir = std::env::temp_dir().join("toradb_highlight_basic");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("Nikola Tesla invented the alternating current induction motor"),
            doc("Marie Curie studied radioactivity"),
        ],
    )
    .expect("add");

    let res = run_table_search(
        &mut dag,
        opts("docs", "alternating current motor", true, 160),
    )
    .unwrap();
    assert!(!res.ids.is_empty());
    assert_eq!(res.snippets.len(), res.ids.len());
    // Top hit is the Tesla doc; its snippet should mark a query term.
    assert!(
        res.snippets[0].contains("<em>"),
        "expected highlighted snippet, got {:?}",
        res.snippets[0]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_highlight_returns_empty_snippets() {
    let dir = std::env::temp_dir().join("toradb_highlight_off");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents("docs", vec![doc("alternating current motor")])
        .expect("add");
    let res = run_table_search(&mut dag, opts("docs", "current", false, 0)).unwrap();
    assert!(res.snippets.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn snippet_len_bounds_length() {
    let dir = std::env::temp_dir().join("toradb_highlight_len");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    let long = format!("{} current {}", "filler ".repeat(80), "filler ".repeat(80));
    dag.add_documents("docs", vec![doc(&long)]).expect("add");
    let res = run_table_search(&mut dag, opts("docs", "current", true, 40)).unwrap();
    // Snippet is windowed well under the full document length.
    assert!(res.snippets[0].len() < long.len());
    assert!(
        res.snippets[0].contains("<em>current</em>"),
        "{:?}",
        res.snippets[0]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
