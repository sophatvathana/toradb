use toradb_engine::{run_table_search, DagRunner, TableSearchOptions};

fn make_dag() -> (DagRunner, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut dag = DagRunner::open(dir.path()).unwrap();
    dag.add_documents(
        "docs",
        vec![
            toradb_index::IngestDoc {
                text: "Nikola Tesla invented the AC motor and the Tesla coil".into(),
                metadata: Default::default(),
                vector: None,
            },
            toradb_index::IngestDoc {
                text: "Thomas Edison pioneered the light bulb and direct current".into(),
                metadata: Default::default(),
                vector: None,
            },
            toradb_index::IngestDoc {
                text: "Marie Curie discovered radium and polonium".into(),
                metadata: Default::default(),
                vector: None,
            },
        ],
    )
    .unwrap();
    (dag, dir)
}

#[test]
fn search_with_explain_populates_provenance() {
    let (mut dag, _dir) = make_dag();
    let result = run_table_search(
        &mut dag,
        TableSearchOptions {
            table: "docs".into(),
            query: "Tesla motor".into(),
            top_k: Some(5),
            offset: None,
            strategy: None,
            explain: true,
            graph_expand: None,
            depth: None,
            query_vector: None,
        },
    )
    .unwrap();

    let prov = result.provenance.expect("provenance should be populated when explain=true");
    assert_eq!(prov.query, "Tesla motor");
    // BM25 candidates should include at least one document.
    assert!(!prov.tier1.bm25_candidates.is_empty(), "expected BM25 candidates");
    // RRF merged tier should be populated.
    assert!(!prov.tier2.rrf_merged.is_empty(), "expected RRF merged candidates");
    // Final ids should match the search results.
    assert_eq!(prov.final_ids, result.ids);
    assert!(prov.total_latency_ms >= 0.0);
}

#[test]
fn search_without_explain_returns_no_provenance() {
    let (mut dag, _dir) = make_dag();
    let result = run_table_search(
        &mut dag,
        TableSearchOptions {
            table: "docs".into(),
            query: "Tesla motor".into(),
            top_k: Some(5),
            offset: None,
            strategy: None,
            explain: false,
            graph_expand: None,
            depth: None,
            query_vector: None,
        },
    )
    .unwrap();

    assert!(result.provenance.is_none(), "provenance should be None when explain=false");
}

#[test]
fn provenance_serialises_to_valid_json() {
    let (mut dag, _dir) = make_dag();
    let result = run_table_search(
        &mut dag,
        TableSearchOptions {
            table: "docs".into(),
            query: "Edison electricity".into(),
            top_k: Some(3),
            offset: None,
            strategy: None,
            explain: true,
            graph_expand: None,
            depth: None,
            query_vector: None,
        },
    )
    .unwrap();

    let prov = result.provenance.unwrap();
    let json = serde_json::to_string_pretty(&prov).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["query"].as_str().is_some());
    assert!(parsed["tier1"].is_object());
    assert!(parsed["tier2"].is_object());
    assert!(parsed["tier3"].is_object());
    assert!(parsed["final_ids"].is_array());
    assert!(parsed["total_latency_ms"].is_number());
}

#[test]
fn provenance_written_to_search_log_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut dag = DagRunner::open(dir.path()).unwrap();
    dag.add_documents(
        "docs",
        vec![toradb_index::IngestDoc {
            text: "Marie Curie discovered radium".into(),
            metadata: Default::default(),
            vector: None,
        }],
    )
    .unwrap();

    run_table_search(
        &mut dag,
        TableSearchOptions {
            table: "docs".into(),
            query: "radium".into(),
            top_k: Some(3),
            offset: None,
            strategy: None,
            explain: true,
            graph_expand: None,
            depth: None,
            query_vector: None,
        },
    )
    .unwrap();

    let log_path = dir.path().join("docs").join("_search_log.ndjson");
    assert!(log_path.exists(), "search log file should be created");
    let content = std::fs::read_to_string(&log_path).unwrap();
    assert!(!content.is_empty(), "search log should contain at least one entry");
    // Each line should be valid JSON.
    for line in content.lines() {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("invalid JSON line in search log: {e}\nline: {line}"));
        assert!(v["query"].is_string());
    }
}
