use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn stream_select_pages_with_offset() {
    let dir = std::env::temp_dir().join("toradb_sql_stream");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    let docs: Vec<_> = (0..6)
        .map(|i| IngestDoc {
            text: format!("Nikola Tesla item {i} motor"),
            metadata: Default::default(),
            vector: None,
        })
        .collect();
    dag.add_documents("docs", docs).expect("add");

    let stmts =
        parse("SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 2").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let mut all_ids = Vec::new();
    let mut offset = 0u32;
    for _ in 0..8 {
        let mut page_sel = sel.clone();
        page_sel.offset = offset;
        let sql_exec::SqlSelectResult::Search(page) =
            sql_exec::run_select(&mut dag, &page_sel).expect("page")
        else {
            panic!("search");
        };
        if page.ids.is_empty() {
            break;
        }
        all_ids.extend(page.ids.iter().copied());
        offset += page.ids.len() as u32;
        if page.ids.len() < 2 {
            break;
        }
    }
    assert!(!all_ids.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
