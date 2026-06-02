use std::collections::HashSet;

use toradb_engine::{count_facets, run_table_search, sql_exec, DagRunner, TableSearchOptions};
use toradb_index::IngestDoc;
use toradb_sql::parse;

fn doc(text: &str, category: &str) -> IngestDoc {
    IngestDoc {
        text: text.into(),
        metadata: [("category".into(), category.into())].into(),
        vector: None,
        sparse: None,
    }
}

fn corpus() -> Vec<IngestDoc> {
    vec![
        doc("Nikola Tesla alternating current motor", "electronics"),
        doc("Nikola Tesla wireless power transmission", "electronics"),
        doc("Nikola Tesla polyphase systems", "electronics"),
        doc("Marie Curie radioactivity research", "books"),
        doc("Marie Curie Nobel prize chemistry", "books"),
    ]
}

fn facet_count(facets: &[toradb_engine::FacetResult], field: &str, value: &str) -> u64 {
    facets
        .iter()
        .find(|f| f.field == field)
        .and_then(|f| f.values.iter().find(|v| v.value == value))
        .map(|v| v.count)
        .unwrap_or(0)
}

#[test]
fn count_facets_counts_over_candidate_set() {
    let dir = std::env::temp_dir().join("toradb_facets_count");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents("docs", corpus()).expect("add");

    // Restrict to docs 0,1,3 — facet counts must reflect only those.
    let candidates: HashSet<u64> = [0u64, 1, 3].into_iter().collect();
    let facets = count_facets(&mut dag, "docs", &["category".into()], &candidates, 20).unwrap();

    assert_eq!(facet_count(&facets, "category", "electronics"), 2);
    assert_eq!(facet_count(&facets, "category", "books"), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn table_search_returns_facets() {
    let dir = std::env::temp_dir().join("toradb_facets_table_search");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents("docs", corpus()).expect("add");

    let result = run_table_search(
        &mut dag,
        TableSearchOptions {
            table: "docs".into(),
            query: "Nikola Tesla Marie Curie".into(),
            top_k: Some(10),
            offset: None,
            strategy: Some("sparse".into()),
            explain: false,
            graph_expand: None,
            depth: None,
            query_vector: None,
            facets: vec!["category".into()],
            facet_top_n: None,
            query_sparse: None,
            bm25_params: None,
            field_boosts: std::collections::HashMap::new(),
            decay: None,
        },
    )
    .unwrap();

    // All five docs match (Tesla or Curie). Facets cover the full matched set.
    assert_eq!(facet_count(&result.facets, "category", "electronics"), 3);
    assert_eq!(facet_count(&result.facets, "category", "books"), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn facets_are_independent_of_pagination() {
    let dir = std::env::temp_dir().join("toradb_facets_pagination");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents("docs", corpus()).expect("add");

    // LIMIT 1: only one hit returned, but facets must still count the full match set.
    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla Marie Curie') \
         FACETS (category) LIMIT 1",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Search(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("search");
    };

    assert_eq!(out.ids.len(), 1, "page should contain a single hit");
    let total: u64 = out
        .facets
        .iter()
        .flat_map(|f| f.values.iter())
        .map(|v| v.count)
        .sum();
    assert_eq!(
        total, 5,
        "facets must count the full matched set, not the page"
    );
    assert_eq!(facet_count(&out.facets, "category", "electronics"), 3);
    assert_eq!(facet_count(&out.facets, "category", "books"), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn facet_top_n_truncates_to_highest_counts() {
    let dir = std::env::temp_dir().join("toradb_facets_top_n");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    // Build a categorical field with skewed cardinality so truncation is observable.
    let mut docs = Vec::new();
    for _ in 0..5 {
        docs.push(doc("Nikola Tesla", "common"));
    }
    for _ in 0..3 {
        docs.push(doc("Nikola Tesla", "medium"));
    }
    docs.push(doc("Nikola Tesla", "rare_a"));
    docs.push(doc("Nikola Tesla", "rare_b"));
    dag.add_documents("docs", docs).expect("add");

    let candidates: HashSet<u64> = (0..10u64).collect();
    let facets = count_facets(&mut dag, "docs", &["category".into()], &candidates, 2).unwrap();

    let values = &facets[0].values;
    assert_eq!(values.len(), 2, "top_n=2 caps the distinct values");
    assert_eq!(values[0].value, "common");
    assert_eq!(values[0].count, 5);
    assert_eq!(values[1].value, "medium");
    assert_eq!(values[1].count, 3);

    let _ = std::fs::remove_dir_all(&dir);
}
