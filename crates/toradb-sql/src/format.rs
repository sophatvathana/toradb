use crate::ast::SelectStmt;

/// Format a parsed retrieval SELECT back into SQL (for materialized view refresh).
pub fn format_select(sel: &SelectStmt) -> String {
    let mut parts = vec!["SELECT id".to_string()];
    parts.push(format!("FROM {}", sel.table));
    if let Some(ref join) = sel.join {
        parts.push(format!(
            "JOIN {} ON {}.{} = {}.{}",
            join.right_table, sel.table, join.left_key, join.right_table, join.right_key
        ));
    }
    if let Some(ref q) = sel.sparse_query {
        let method = sel.sparse.as_deref().unwrap_or("bm25");
        parts.push(format!(
            "SPARSE SEARCH body {}('{q}')",
            method.to_uppercase()
        ));
    }
    if sel.vector {
        if let Some(ref v) = sel.vector_query {
            let nums: Vec<String> = v.iter().map(|x| x.to_string()).collect();
            parts.push(format!(
                "VECTOR SEARCH embedding ANN([{}])",
                nums.join(", ")
            ));
        } else if let Some(ref t) = sel.vector_text {
            parts.push(format!("VECTOR SEARCH embedding ANN('{t}')"));
        } else {
            parts.push("VECTOR SEARCH embedding ANN('')".into());
        }
    }
    if let Some(ref ob) = sel.order_by {
        parts.push(format!(
            "ORDER BY {} {}",
            ob.column,
            if ob.descending { "DESC" } else { "ASC" }
        ));
    }
    if !sel.group_by.is_empty() {
        parts.push(format!("GROUP BY {}", sel.group_by.join(", ")));
    }
    parts.push(format!("LIMIT {}", sel.limit));
    if sel.offset > 0 {
        parts.push(format!("OFFSET {}", sel.offset));
    }
    parts.join(" ")
}
