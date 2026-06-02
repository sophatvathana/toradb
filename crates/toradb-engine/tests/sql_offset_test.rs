use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn sql_search_offset_skips_top_hits() {
    let dir = std::env::temp_dir().join("toradb_sql_offset");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        let docs: Vec<IngestDoc> = (0..5u64)
            .map(|i| IngestDoc {
                text: format!("Nikola Tesla document number {i}"),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            })
            .collect();
        dag.add_documents("docs", docs).expect("add");
    }

    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla') ORDER BY SCORE DESC LIMIT 2 OFFSET 2",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let mut dag = DagRunner::open(&dir).expect("reopen");

    let stmts_all = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla') ORDER BY SCORE DESC LIMIT 5 OFFSET 0",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel_all) = &stmts_all[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Search(all) =
        sql_exec::run_select(&mut dag, sel_all).expect("all")
    else {
        panic!("search");
    };

    let sql_exec::SqlSelectResult::Search(page) =
        sql_exec::run_select(&mut dag, sel).expect("page")
    else {
        panic!("search");
    };

    assert_eq!(page.ids.len(), 2);
    assert!(page.ids[0] != page.ids[1]);
    for id in &page.ids {
        assert!(
            all.ids.contains(id),
            "offset page id {id} should appear in full result"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
