use std::collections::HashMap;

use toradb_engine::{run_table_search, DagRunner, TableSearchOptions};
use toradb_index::IngestDoc;

fn doc(text: &str, meta: &[(&str, &str)]) -> IngestDoc {
    IngestDoc {
        text: text.into(),
        metadata: meta
            .iter()
            .map(|&(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        vector: None,
        sparse: None,
    }
}

fn base_opts(table: &str, query: &str) -> TableSearchOptions {
    TableSearchOptions {
        table: table.into(),
        query: query.into(),
        top_k: Some(10),
        offset: None,
        strategy: Some("sparse".into()),
        explain: true,
        graph_expand: None,
        depth: None,
        query_vector: None,
        facets: Vec::new(),
        facet_top_n: None,
        query_sparse: None,
        bm25_params: None,
        field_boosts: HashMap::new(),
        decay: None,
        highlight: false,
        snippet_len: 0,
    }
}

#[test]
fn field_boost_promotes_matching_doc() {
    let dir = std::env::temp_dir().join("toradb_knobs_boost");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("tesla tesla motor", &[]),
            doc("tesla motor", &[("editor_pick", "yes")]),
        ],
    )
    .expect("add");

    let baseline = run_table_search(&mut dag, base_opts("docs", "tesla motor")).unwrap();
    assert_eq!(baseline.ids[0], 0, "baseline: higher-tf doc first");

    let mut opts = base_opts("docs", "tesla motor");
    opts.field_boosts = [("editor_pick".to_string(), 5.0)].into();
    let boosted = run_table_search(&mut dag, opts).unwrap();
    assert_eq!(boosted.ids[0], 1, "boost should promote the editor pick");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn temporal_decay_prefers_recent() {
    let dir = std::env::temp_dir().join("toradb_knobs_decay");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("tesla motor", &[("published", "2020-01-01")]), // old
            doc("tesla motor", &[("published", "2999-01-01")]), // far future => ~no decay
        ],
    )
    .expect("add");

    let mut opts = base_opts("docs", "tesla motor");
    opts.decay = Some(("published".to_string(), 30.0));
    let res = run_table_search(&mut dag, opts).unwrap();
    assert_eq!(res.ids[0], 1, "recent doc should rank first under decay");
    assert!(res.scores[0] >= res.scores[1]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_knobs_is_identity() {
    let dir = std::env::temp_dir().join("toradb_knobs_identity");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![doc("tesla tesla motor", &[]), doc("tesla motor", &[])],
    )
    .expect("add");
    let a = run_table_search(&mut dag, base_opts("docs", "tesla motor")).unwrap();
    let b = run_table_search(&mut dag, base_opts("docs", "tesla motor")).unwrap();
    assert_eq!(a.ids, b.ids);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn provenance_records_score_breakdown() {
    let dir = std::env::temp_dir().join("toradb_knobs_prov");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents("docs", vec![doc("tesla motor", &[("editor_pick", "yes")])])
        .expect("add");
    let mut opts = base_opts("docs", "tesla motor");
    opts.field_boosts = [("editor_pick".to_string(), 3.0)].into();
    let res = run_table_search(&mut dag, opts).unwrap();
    let prov = res.provenance.expect("provenance");
    assert!(
        !prov.score_breakdown.is_empty(),
        "breakdown should be recorded"
    );
    let row = &prov.score_breakdown[0];
    assert!((row.boost - 3.0).abs() < 1e-6);
    assert!((row.final_score - row.base * row.boost * row.decay).abs() < 1e-4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn knobs_rerank_full_window_under_small_limit() {
    let dir = std::env::temp_dir().join("toradb_knobs_window");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    let mut docs = Vec::new();
    for _ in 0..10 {
        docs.push(doc("tesla tesla tesla motor", &[]));
    }
    docs.push(doc("tesla motor", &[("editor_pick", "yes")]));
    dag.add_documents("docs", docs).expect("add");

    let mut opts = base_opts("docs", "tesla motor");
    opts.top_k = Some(1);
    opts.field_boosts = [("editor_pick".to_string(), 100.0)].into();
    let res = run_table_search(&mut dag, opts).unwrap();
    assert_eq!(res.ids.len(), 1);
    assert_eq!(res.ids[0], 10, "boosted doc should win despite small top_k");

    let _ = std::fs::remove_dir_all(&dir);
}
