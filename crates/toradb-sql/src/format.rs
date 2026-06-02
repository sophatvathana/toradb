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
        let mut args = format!("'{q}'");
        if let Some(k1) = sel.bm25_k1 {
            args.push_str(&format!(", k1={k1}"));
        }
        if let Some(b) = sel.bm25_b {
            args.push_str(&format!(", b={b}"));
        }
        parts.push(format!("SPARSE SEARCH body {}({args})", method.to_uppercase()));
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
    if !sel.facets.is_empty() {
        parts.push(format!("FACETS ({})", sel.facets.join(", ")));
    }
    let mut boost_fields: Vec<(&String, &f32)> = sel.field_boosts.iter().collect();
    boost_fields.sort_by(|a, b| a.0.cmp(b.0));
    for (field, factor) in boost_fields {
        parts.push(format!("BOOST({field}, {factor})"));
    }
    if let Some((ref field, half_life)) = sel.decay {
        parts.push(format!("DECAY({field}, half_life={half_life})"));
    }
    parts.push(format!("LIMIT {}", sel.limit));
    if sel.offset > 0 {
        parts.push(format!("OFFSET {}", sel.offset));
    }
    parts.join(" ")
}
