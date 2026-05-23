use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

fn unit_vector(i: u64, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0; dim];
    v[i as usize % dim] = 1.0;
    v
}

#[test]
fn sql_vector_search_uses_diskann_sidecar_when_present() {
    let dir = std::env::temp_dir().join("toradb_sql_diskann");
    let _ = std::fs::remove_dir_all(&dir);
    let dim = 8;

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        let docs: Vec<IngestDoc> = (0..40u64)
            .map(|i| IngestDoc {
                text: format!("doc {i}"),
                metadata: Default::default(),
                vector: Some(unit_vector(i, dim)),
            })
            .collect();
        dag.add_documents("emb", docs).expect("add");
    }

    std::fs::remove_file(dir.join("emb/indexes/hnsw.bin")).ok();

    let mut dag = DagRunner::open(&dir).expect("reopen");
    assert!(dag.table_has_diskann_sidecar("emb"));

    let q = unit_vector(39, dim);
    let ann = format!(
        "[{}]",
        q.iter()
            .map(|x| {
                if x.fract() == 0.0 {
                    format!("{:.1}", x)
                } else {
                    x.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    );
    let sql = format!("SELECT id FROM emb VECTOR SEARCH embedding ANN({ann}) LIMIT 5");
    let stmts = parse(&sql).unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Search(out) = sql_exec::run_select(&mut dag, sel).expect("run")
    else {
        panic!("expected search");
    };
    assert!(!out.ids.is_empty());
    assert!(out.ids.iter().take(5).any(|&id| id == 39));

    let _ = std::fs::remove_dir_all(&dir);
}
