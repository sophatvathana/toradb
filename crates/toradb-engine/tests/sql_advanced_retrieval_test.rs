use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn sql_hyde_crag_graph_and_fusion_parse_and_run() {
    let dir = std::env::temp_dir().join("toradb_sql_advanced");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    let docs: Vec<IngestDoc> = (0..20u64)
        .map(|i| IngestDoc {
            text: format!("Nikola Tesla motor coil {i}"),
            metadata: Default::default(),
            vector: None,
            sparse: None,
        })
        .collect();
    dag.add_documents("docs", docs).expect("add");

    let sql = "SELECT id FROM docs \
        SPARSE SEARCH body BM25('Tesla motor') \
        HYDE CRAG GRAPH EXPAND 2 FUSION K = 40 LIMIT 5";
    let stmts = parse(sql).expect("parse");
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert!(sel.hyde);
    assert!(sel.crag);
    assert!(sel.graph_expand);
    assert_eq!(sel.graph_depth, 2);
    assert_eq!(sel.fusion_k, 40);

    let sql_exec::SqlSelectResult::Search(out) = sql_exec::run_select(&mut dag, sel).expect("run")
    else {
        panic!("search");
    };
    assert!(!out.ids.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
