use crate::ast::*;
use crate::lexer::{tokenize, Token};

fn ident_at(tokens: &[Token], i: usize) -> Option<String> {
    match tokens.get(i) {
        Some(Token::Ident(s)) => Some(s.clone()),
        _ => None,
    }
}

fn expect_ident(tokens: &[Token], i: &mut usize, word: &str) -> Result<(), String> {
    let Some(got) = ident_at(tokens, *i) else {
        return Err(format!("expected {word}"));
    };
    if got != word {
        return Err(format!("expected {word}, got {got}"));
    }
    *i += 1;
    Ok(())
}

fn parse_column_defs(tokens: &[Token], i: &mut usize) -> Result<Vec<(String, String)>, String> {
    let mut columns = Vec::new();
    if !matches!(tokens.get(*i), Some(Token::LParen)) {
        return Ok(columns);
    }
    *i += 1; // consume '('
    loop {
        if matches!(tokens.get(*i), Some(Token::RParen)) {
            *i += 1;
            break;
        }
        let Some(name) = ident_at(tokens, *i) else {
            return Err("expected column name in CREATE TABLE".into());
        };
        let name = name.to_lowercase();
        *i += 1;
        let mut ty = ident_at(tokens, *i).unwrap_or_else(|| "TEXT".into());
        if ident_at(tokens, *i).is_some() {
            *i += 1;
        }
        consume_type_precision(tokens, i, &mut ty);
        columns.push((name, ty));
        match tokens.get(*i) {
            Some(Token::Comma) => {
                *i += 1;
                continue;
            }
            Some(Token::RParen) => {
                *i += 1;
                break;
            }
            _ => return Err("expected ',' or ')' in CREATE TABLE column list".into()),
        }
    }
    Ok(columns)
}

fn consume_type_precision(tokens: &[Token], i: &mut usize, ty: &mut String) {
    let (open, close, close_tok): (char, char, fn(&Token) -> bool) = match tokens.get(*i) {
        Some(Token::LParen) => ('(', ')', |t| matches!(t, Token::RParen)),
        Some(Token::LBracket) => ('[', ']', |t| matches!(t, Token::RBracket)),
        _ => return,
    };
    ty.push(open);
    *i += 1;
    while let Some(tok) = tokens.get(*i) {
        if close_tok(tok) {
            break;
        }
        match tok {
            Token::Number(n) => ty.push_str(&n.to_string()),
            Token::Comma => ty.push(','),
            Token::Ident(s) => ty.push_str(s),
            _ => {}
        }
        *i += 1;
    }
    if tokens.get(*i).map(close_tok).unwrap_or(false) {
        ty.push(close);
        *i += 1;
    }
}

fn parse_type_at(tokens: &[Token], i: &mut usize) -> Result<String, String> {
    let mut ty = ident_at(tokens, *i).ok_or("expected column type")?;
    *i += 1;
    consume_type_precision(tokens, i, &mut ty);
    Ok(ty.to_ascii_lowercase())
}

fn parse_aggregate(tokens: &[Token], i: &mut usize) -> Result<SelectExpr, String> {
    let func = match ident_at(tokens, *i).as_deref() {
        Some("COUNT") => AggFunc::CountStar,
        Some("SUM") => AggFunc::Sum,
        Some("AVG") => AggFunc::Avg,
        Some("MIN") => AggFunc::Min,
        Some("MAX") => AggFunc::Max,
        Some(other) => return Err(format!("unknown aggregate {other}")),
        None => return Err("expected aggregate function".into()),
    };
    *i += 1;
    let mut column = None;
    if matches!(tokens.get(*i), Some(Token::LParen)) {
        *i += 1;
        if matches!(tokens.get(*i), Some(Token::RParen)) {
            if !matches!(func, AggFunc::CountStar) {
                return Err("aggregate requires column argument".into());
            }
        } else if matches!(tokens.get(*i), Some(Token::Star)) {
            if !matches!(func, AggFunc::CountStar) {
                return Err("aggregate * only supported for COUNT".into());
            }
            *i += 1;
        } else if let Some(col) = ident_at(tokens, *i) {
            column = Some(col.to_lowercase());
            *i += 1;
        } else {
            return Err("aggregate requires column or )".into());
        }
        if !matches!(tokens.get(*i), Some(Token::RParen)) {
            return Err("expected ) after aggregate".into());
        }
        *i += 1;
    }
    if !matches!(func, AggFunc::CountStar) && column.is_none() {
        return Err("aggregate requires a column argument".into());
    }
    Ok(SelectExpr::Aggregate { func, column })
}

fn parse_select_exprs(tokens: &[Token], i: &mut usize) -> Result<Vec<SelectExpr>, String> {
    let mut items = Vec::new();
    loop {
        if matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "FROM") {
            break;
        }
        if matches!(
            tokens.get(*i),
            Some(Token::Ident(k)) if matches!(k.as_str(), "COUNT" | "SUM" | "AVG" | "MIN" | "MAX")
        ) {
            items.push(parse_aggregate(tokens, i)?);
        } else if matches!(tokens.get(*i), Some(Token::Star)) {
            items.push(SelectExpr::All);
            *i += 1;
        } else if let Some(col) = ident_at(tokens, *i) {
            items.push(SelectExpr::Column(col.to_lowercase()));
            *i += 1;
        } else {
            return Err("expected select expression".into());
        }
        if matches!(tokens.get(*i), Some(Token::Comma)) {
            *i += 1;
            continue;
        }
        if matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "FROM") {
            break;
        }
        return Err("expected comma or FROM in select list".into());
    }
    Ok(items)
}

fn parse_literal(tokens: &[Token], i: &mut usize) -> Result<String, String> {
    match tokens.get(*i) {
        Some(Token::String(s)) => {
            *i += 1;
            Ok(s.clone())
        }
        Some(Token::Ident(s)) => {
            *i += 1;
            Ok(s.to_lowercase())
        }
        Some(Token::Number(n)) => {
            *i += 1;
            Ok(n.to_string())
        }
        _ => Err("expected literal value".into()),
    }
}

fn parse_compare_op(token: &Token) -> Option<CompareOp> {
    match token {
        Token::Eq => Some(CompareOp::Eq),
        Token::Ne => Some(CompareOp::Ne),
        Token::Lt => Some(CompareOp::Lt),
        Token::Lte => Some(CompareOp::Lte),
        Token::Gt => Some(CompareOp::Gt),
        Token::Gte => Some(CompareOp::Gte),
        _ => None,
    }
}

fn where_clause_boundary(tokens: &[Token], i: usize) -> bool {
    matches!(tokens.get(i), Some(Token::Ident(k)) if matches!(
        k.as_str(),
        "GROUP" | "HAVING" | "ORDER" | "LIMIT" | "OFFSET" | "SPARSE" | "VECTOR" | "JOIN"
            | "HYDE" | "CRAG" | "GRAPH" | "FUSION" | "DISTRIBUTED" | "STREAM" | "EXPLAIN"
            | "FACETS" | "BOOST" | "DECAY" | "HIGHLIGHT"
    ))
}

fn parse_leaf_predicate(tokens: &[Token], i: &mut usize) -> Result<WherePred, String> {
    let column = ident_at(tokens, *i)
        .ok_or("predicate requires column name".to_string())?
        .to_lowercase();
    *i += 1;

    if matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "IN") {
        *i += 1;
        if !matches!(tokens.get(*i), Some(Token::LParen)) {
            return Err("IN requires (".into());
        }
        *i += 1;
        let mut values = Vec::new();
        loop {
            if matches!(tokens.get(*i), Some(Token::RParen)) {
                *i += 1;
                break;
            }
            values.push(parse_literal(tokens, i)?);
            if matches!(tokens.get(*i), Some(Token::Comma)) {
                *i += 1;
                continue;
            }
            if matches!(tokens.get(*i), Some(Token::RParen)) {
                *i += 1;
                break;
            }
            return Err("expected , or ) in IN list".into());
        }
        if values.is_empty() {
            return Err("IN requires at least one value".into());
        }
        return Ok(WherePred::In { column, values });
    }

    let mut negated = false;
    if matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "NOT")
        && matches!(tokens.get(*i + 1), Some(Token::Ident(k)) if k == "BETWEEN" || k == "LIKE")
    {
        negated = true;
        *i += 1;
    }
    if matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "BETWEEN") {
        *i += 1;
        let low = parse_literal(tokens, i)?;
        expect_ident(tokens, i, "AND")?;
        let high = parse_literal(tokens, i)?;
        return Ok(WherePred::Between {
            column,
            low,
            high,
            negated,
        });
    }
    if matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "LIKE") {
        *i += 1;
        let pattern = parse_literal(tokens, i)?;
        return Ok(WherePred::Like {
            column,
            pattern,
            negated,
        });
    }

    let op = tokens
        .get(*i)
        .and_then(parse_compare_op)
        .ok_or("predicate requires comparison operator".to_string())?;
    *i += 1;
    let value = parse_literal(tokens, i)?;
    Ok(WherePred::Compare { column, op, value })
}

fn parse_primary_predicate(tokens: &[Token], i: &mut usize) -> Result<WherePred, String> {
    if matches!(tokens.get(*i), Some(Token::LParen)) {
        *i += 1;
        let pred = parse_or_predicate(tokens, i)?;
        if !matches!(tokens.get(*i), Some(Token::RParen)) {
            return Err("expected ) after grouped predicate".into());
        }
        *i += 1;
        return Ok(pred);
    }
    parse_leaf_predicate(tokens, i)
}

fn parse_and_predicate(tokens: &[Token], i: &mut usize) -> Result<WherePred, String> {
    let mut left = parse_primary_predicate(tokens, i)?;
    while matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "AND")
        && !where_clause_boundary(tokens, *i + 1)
    {
        *i += 1;
        let right = parse_primary_predicate(tokens, i)?;
        left = match left {
            WherePred::And(mut parts) => {
                parts.push(right);
                WherePred::And(parts)
            }
            _ => WherePred::And(vec![left, right]),
        };
    }
    Ok(left)
}

fn parse_or_predicate(tokens: &[Token], i: &mut usize) -> Result<WherePred, String> {
    let mut left = parse_and_predicate(tokens, i)?;
    while matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "OR") {
        *i += 1;
        let right = parse_and_predicate(tokens, i)?;
        left = match left {
            WherePred::Or(mut parts) => {
                parts.push(right);
                WherePred::Or(parts)
            }
            _ => WherePred::Or(vec![left, right]),
        };
    }
    Ok(left)
}

fn parse_predicate_clause(
    tokens: &[Token],
    i: &mut usize,
    clause_name: &str,
) -> Result<WherePred, String> {
    expect_ident(tokens, i, clause_name)?;
    parse_or_predicate(tokens, i)
}

fn parse_where_clause(tokens: &[Token], i: &mut usize) -> Result<WherePred, String> {
    parse_predicate_clause(tokens, i, "WHERE")
}

fn parse_having_clause(tokens: &[Token], i: &mut usize) -> Result<WherePred, String> {
    parse_predicate_clause(tokens, i, "HAVING")
}

fn parse_group_by_clause(tokens: &[Token], i: &mut usize) -> Result<Vec<String>, String> {
    expect_ident(tokens, i, "GROUP")?;
    expect_ident(tokens, i, "BY")?;
    let mut out = Vec::new();
    loop {
        let Some(col) = ident_at(tokens, *i) else {
            return Err("GROUP BY requires at least one column".into());
        };
        out.push(col.to_lowercase());
        *i += 1;
        if matches!(tokens.get(*i), Some(Token::Comma)) {
            *i += 1;
            continue;
        }
        break;
    }
    Ok(out)
}

fn parse_facets_clause(tokens: &[Token], i: &mut usize) -> Result<Vec<String>, String> {
    expect_ident(tokens, i, "FACETS")?;
    if !matches!(tokens.get(*i), Some(Token::LParen)) {
        return Err("FACETS requires (col1, col2, ...)".into());
    }
    *i += 1;
    let mut out = Vec::new();
    loop {
        if matches!(tokens.get(*i), Some(Token::RParen)) {
            *i += 1;
            break;
        }
        let Some(col) = ident_at(tokens, *i) else {
            return Err("FACETS requires column names".into());
        };
        out.push(col.to_lowercase());
        *i += 1;
        match tokens.get(*i) {
            Some(Token::Comma) => {
                *i += 1;
                continue;
            }
            Some(Token::RParen) => {
                *i += 1;
                break;
            }
            _ => return Err("expected , or ) in FACETS list".into()),
        }
    }
    if out.is_empty() {
        return Err("FACETS requires at least one column".into());
    }
    Ok(out)
}

fn parse_highlight_clause(tokens: &[Token], i: &mut usize) -> Result<Option<u32>, String> {
    expect_ident(tokens, i, "HIGHLIGHT")?;
    if !matches!(tokens.get(*i), Some(Token::LParen)) {
        return Ok(None);
    }
    *i += 1;
    let len = match tokens.get(*i) {
        Some(Token::Number(n)) => {
            *i += 1;
            Some(*n)
        }
        _ => return Err("HIGHLIGHT(len) requires a number".into()),
    };
    if !matches!(tokens.get(*i), Some(Token::RParen)) {
        return Err("HIGHLIGHT requires closing )".into());
    }
    *i += 1;
    Ok(len)
}

/// `BOOST(field, factor)` — single clause; the caller loops for repeats.
fn parse_boost_clause(tokens: &[Token], i: &mut usize) -> Result<(String, f32), String> {
    expect_ident(tokens, i, "BOOST")?;
    if !matches!(tokens.get(*i), Some(Token::LParen)) {
        return Err("BOOST requires (field, factor)".into());
    }
    *i += 1;
    let field = ident_at(tokens, *i)
        .ok_or("BOOST requires a field name")?
        .to_lowercase();
    *i += 1;
    if !matches!(tokens.get(*i), Some(Token::Comma)) {
        return Err("BOOST requires (field, factor)".into());
    }
    *i += 1;
    let factor = number_at_f32(tokens, i).ok_or("BOOST factor must be a number")?;
    if !matches!(tokens.get(*i), Some(Token::RParen)) {
        return Err("BOOST requires closing )".into());
    }
    *i += 1;
    Ok((field, factor))
}

/// `DECAY(field, half_life=days)` - temporal decay.
fn parse_decay_clause(tokens: &[Token], i: &mut usize) -> Result<(String, f32), String> {
    expect_ident(tokens, i, "DECAY")?;
    if !matches!(tokens.get(*i), Some(Token::LParen)) {
        return Err("DECAY requires (field, half_life=N)".into());
    }
    *i += 1;
    let field = ident_at(tokens, *i)
        .ok_or("DECAY requires a field name")?
        .to_lowercase();
    *i += 1;
    if !matches!(tokens.get(*i), Some(Token::Comma)) {
        return Err("DECAY requires (field, half_life=N)".into());
    }
    *i += 1;
    // optional `half_life` keyword
    if matches!(tokens.get(*i), Some(Token::Ident(k)) if k.eq_ignore_ascii_case("half_life")) {
        *i += 1;
        if matches!(tokens.get(*i), Some(Token::Eq)) {
            *i += 1;
        }
    }
    let half_life = number_at_f32(tokens, i).ok_or("DECAY half_life must be a number")?;
    if !matches!(tokens.get(*i), Some(Token::RParen)) {
        return Err("DECAY requires closing )".into());
    }
    *i += 1;
    Ok((field, half_life))
}

fn parse_float_list(tokens: &[Token], i: &mut usize) -> Result<Vec<f32>, String> {
    if !matches!(tokens.get(*i), Some(Token::LBracket)) {
        return Err("ANN vector literal requires [...]".into());
    }
    *i += 1;
    let mut out = Vec::new();
    loop {
        match tokens.get(*i) {
            Some(Token::RBracket) => {
                *i += 1;
                if out.is_empty() {
                    return Err("empty vector literal".into());
                }
                return Ok(out);
            }
            Some(Token::Comma) => {
                *i += 1;
            }
            Some(Token::Number(n)) => {
                out.push(*n as f32);
                *i += 1;
            }
            Some(Token::Float(f)) => {
                out.push(*f);
                *i += 1;
            }
            _ => return Err("expected number in vector literal".into()),
        }
    }
}

fn parse_vector_search(
    tokens: &[Token],
    i: &mut usize,
) -> Result<(bool, Option<Vec<f32>>, Option<String>), String> {
    expect_ident(tokens, i, "VECTOR")?;
    expect_ident(tokens, i, "SEARCH")?;
    if ident_at(tokens, *i).is_some() {
        *i += 1;
    }
    if let Some(method) = ident_at(tokens, *i) {
        let m = method.to_lowercase();
        if m == "ann" || m == "dot" || m == "hnsw" {
            *i += 1;
        }
    }
    let mut vector_query = None;
    let mut vector_text = None;
    if matches!(tokens.get(*i), Some(Token::LParen)) {
        *i += 1;
        if matches!(tokens.get(*i), Some(Token::LBracket)) {
            vector_query = Some(parse_float_list(tokens, i)?);
        } else if let Some(Token::String(q)) = tokens.get(*i) {
            if q.contains(',') {
                let parsed: Result<Vec<f32>, _> =
                    q.split(',').map(|s| s.trim().parse::<f32>()).collect();
                vector_query = Some(parsed.map_err(|_| "invalid ANN vector string".to_string())?);
            } else {
                vector_text = Some(q.clone());
            }
            *i += 1;
        }
        if matches!(tokens.get(*i), Some(Token::RParen)) {
            *i += 1;
        }
    }
    Ok((true, vector_query, vector_text))
}

fn number_at_f32(tokens: &[Token], i: &mut usize) -> Option<f32> {
    match tokens.get(*i) {
        Some(Token::Float(f)) => {
            *i += 1;
            Some(*f)
        }
        Some(Token::Number(n)) => {
            *i += 1;
            Some(*n as f32)
        }
        _ => None,
    }
}

fn parse_sparse_search(
    tokens: &[Token],
    i: &mut usize,
) -> Result<(Option<String>, Option<String>, Option<f32>, Option<f32>), String> {
    // SPARSE SEARCH <col> BM25 ( 'query' [, k1=1.5] [, b=0.6] )
    expect_ident(tokens, i, "SPARSE")?;
    expect_ident(tokens, i, "SEARCH")?;
    if ident_at(tokens, *i).is_some() {
        *i += 1; // column
    }
    let method = ident_at(tokens, *i).map(|m| m.to_lowercase());
    if method.as_deref().is_some_and(|m| {
        matches!(
            m,
            "bm25" | "splade" | "seismic" | "sparse" | "text" | "ann" | "dot" | "hnsw"
        )
    }) {
        *i += 1;
    } else if method.is_some() {
        *i += 1;
    }
    let mut query = None;
    let mut k1 = None;
    let mut b = None;
    if matches!(tokens.get(*i), Some(Token::LParen)) {
        *i += 1;
        if let Some(Token::String(q)) = tokens.get(*i) {
            query = Some(q.clone());
            *i += 1;
        }
        while matches!(tokens.get(*i), Some(Token::Comma)) {
            *i += 1;
            let name = ident_at(tokens, *i).map(|s| s.to_lowercase());
            match name.as_deref() {
                Some(p @ ("k1" | "b")) => {
                    let is_k1 = p == "k1";
                    *i += 1;
                    if matches!(tokens.get(*i), Some(Token::Eq)) {
                        *i += 1;
                    }
                    let val = number_at_f32(tokens, i).ok_or("BM25 k1/b requires a number")?;
                    if is_k1 {
                        k1 = Some(val);
                    } else {
                        b = Some(val);
                    }
                }
                _ => return Err("BM25 only accepts k1= and b= params".into()),
            }
        }
        if matches!(tokens.get(*i), Some(Token::RParen)) {
            *i += 1;
        }
    }
    Ok((method, query, k1, b))
}

fn parse_qualified_column(tokens: &[Token], i: &mut usize) -> Result<(String, String), String> {
    let table = ident_at(tokens, *i)
        .ok_or("expected table.column in JOIN ON")?
        .to_lowercase();
    *i += 1;
    if !matches!(tokens.get(*i), Some(Token::Dot)) {
        return Err("expected . after table name in JOIN ON".into());
    }
    *i += 1;
    let column = ident_at(tokens, *i)
        .ok_or("expected column after table. in JOIN ON")?
        .to_lowercase();
    *i += 1;
    Ok((table, column))
}

fn parse_from_clause(
    tokens: &[Token],
    i: &mut usize,
) -> Result<(String, Option<JoinClause>), String> {
    let (_ns, left_table) = parse_qualified_table(tokens, i)?;
    if !matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "JOIN") {
        return Ok((left_table, None));
    }
    *i += 1;
    let right_table = ident_at(tokens, *i)
        .ok_or("table name after JOIN")?
        .to_lowercase();
    *i += 1;
    expect_ident(tokens, i, "ON")?;
    let (left_qual, left_key) = parse_qualified_column(tokens, i)?;
    if !matches!(tokens.get(*i), Some(Token::Eq)) {
        return Err("JOIN ON requires =".into());
    }
    *i += 1;
    let (right_qual, right_key) = parse_qualified_column(tokens, i)?;
    if left_qual != left_table {
        return Err(format!(
            "JOIN ON left side must use {left_table}, got {left_qual}"
        ));
    }
    if right_qual != right_table {
        return Err(format!(
            "JOIN ON right side must use {right_table}, got {right_qual}"
        ));
    }
    Ok((
        left_table,
        Some(JoinClause {
            right_table,
            left_key,
            right_key,
        }),
    ))
}

pub fn parse_select_stmt(tokens: &[Token], i: &mut usize) -> Result<SelectStmt, String> {
    expect_ident(tokens, i, "SELECT")?;
    let distinct = if matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "DISTINCT") {
        *i += 1;
        true
    } else {
        false
    };
    let select_items = parse_select_exprs(tokens, i)?;
    if !matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "FROM") {
        return Err("SELECT requires FROM".into());
    }
    *i += 1;
    let (table, join) = parse_from_clause(tokens, i)?;
    let mut sparse = None;
    let mut sparse_query = None;
    let mut vector = false;
    let mut vector_query = None;
    let mut vector_text = None;
    let mut limit = 20;
    let mut offset = 0u32;
    let mut order_by = None;
    let mut distributed = false;
    let mut hyde = false;
    let mut crag = false;
    let mut graph_expand = false;
    let mut graph_depth = 1u32;
    let mut fusion_k = 60u32;
    let mut stream = false;
    let mut explain = false;
    let mut group_by = Vec::new();
    let mut where_clause = None;
    let mut having_clause = None;
    let mut facets = Vec::new();
    let mut bm25_k1: Option<f32> = None;
    let mut bm25_b: Option<f32> = None;
    let mut field_boosts: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
    let mut decay: Option<(String, f32)> = None;
    let mut highlight = false;
    let mut snippet_len: Option<u32> = None;
    while *i < tokens.len() && !matches!(tokens.get(*i), Some(Token::Eof) | Some(Token::RParen)) {
        match tokens.get(*i) {
            Some(Token::Ident(k)) if k == "HYDE" => {
                hyde = true;
                *i += 1;
            }
            Some(Token::Ident(k)) if k == "CRAG" => {
                crag = true;
                *i += 1;
            }
            Some(Token::Ident(k)) if k == "GRAPH" => {
                *i += 1;
                expect_ident(tokens, i, "EXPAND")?;
                graph_expand = true;
                if let Some(Token::Number(n)) = tokens.get(*i) {
                    graph_depth = *n;
                    *i += 1;
                }
            }
            Some(Token::Ident(k)) if k == "FUSION" => {
                *i += 1;
                expect_ident(tokens, i, "K")?;
                if matches!(tokens.get(*i), Some(Token::Eq)) {
                    *i += 1;
                }
                if let Some(Token::Number(n)) = tokens.get(*i) {
                    fusion_k = *n;
                    *i += 1;
                }
            }
            Some(Token::Ident(k)) if k == "DISTRIBUTED" => {
                distributed = true;
                *i += 1;
            }
            Some(Token::Ident(k)) if k == "STREAM" => {
                stream = true;
                *i += 1;
            }
            Some(Token::Ident(k)) if k == "EXPLAIN" => {
                explain = true;
                *i += 1;
            }
            Some(Token::Ident(k)) if k == "SPARSE" => {
                let (method, query, k1, b) = parse_sparse_search(tokens, i)?;
                sparse = method.or(Some("bm25".into()));
                sparse_query = query;
                if k1.is_some() {
                    bm25_k1 = k1;
                }
                if b.is_some() {
                    bm25_b = b;
                }
            }
            Some(Token::Ident(k)) if k == "VECTOR" => {
                let (v, vq, vt) = parse_vector_search(tokens, i)?;
                vector = v;
                vector_query = vq;
                vector_text = vt;
            }
            Some(Token::Ident(k)) if k == "LIMIT" => {
                *i += 1;
                if let Some(Token::Number(n)) = tokens.get(*i) {
                    limit = *n;
                    *i += 1;
                }
            }
            Some(Token::Ident(k)) if k == "OFFSET" => {
                *i += 1;
                if let Some(Token::Number(n)) = tokens.get(*i) {
                    offset = *n;
                    *i += 1;
                }
            }
            Some(Token::Ident(k)) if k == "ORDER" => {
                order_by = Some(parse_order_by(tokens, i)?);
            }
            Some(Token::Ident(k)) if k == "GROUP" => {
                group_by = parse_group_by_clause(tokens, i)?;
            }
            Some(Token::Ident(k)) if k == "WHERE" => {
                where_clause = Some(parse_where_clause(tokens, i)?);
            }
            Some(Token::Ident(k)) if k == "HAVING" => {
                having_clause = Some(parse_having_clause(tokens, i)?);
            }
            Some(Token::Ident(k)) if k == "FACETS" => {
                facets = parse_facets_clause(tokens, i)?;
            }
            Some(Token::Ident(k)) if k == "BOOST" => {
                let (field, factor) = parse_boost_clause(tokens, i)?;
                field_boosts.insert(field, factor);
            }
            Some(Token::Ident(k)) if k == "DECAY" => {
                decay = Some(parse_decay_clause(tokens, i)?);
            }
            Some(Token::Ident(k)) if k == "HIGHLIGHT" => {
                snippet_len = parse_highlight_clause(tokens, i)?;
                highlight = true;
            }
            Some(Token::Semi) | Some(Token::Eof) | Some(Token::RParen) => break,
            _ => *i += 1,
        }
    }
    Ok(SelectStmt {
        ctes: Vec::new(),
        table,
        join,
        select_items,
        distinct,
        sparse,
        sparse_query,
        vector,
        vector_query,
        vector_text,
        limit,
        offset,
        order_by,
        distributed,
        hyde,
        crag,
        graph_expand,
        graph_depth,
        fusion_k,
        stream,
        explain,
        group_by,
        where_clause,
        having_clause,
        facets,
        bm25_k1,
        bm25_b,
        field_boosts,
        decay,
        highlight,
        snippet_len,
    })
}

fn parse_ctes(tokens: &[Token], i: &mut usize) -> Result<Vec<Cte>, String> {
    expect_ident(tokens, i, "WITH")?;
    let mut out = Vec::new();
    loop {
        let name = ident_at(tokens, *i)
            .ok_or("CTE name after WITH")?
            .to_lowercase();
        *i += 1;
        expect_ident(tokens, i, "AS")?;
        if !matches!(tokens.get(*i), Some(Token::LParen)) {
            return Err("CTE requires AS (<select>)".into());
        }
        *i += 1;
        let query = parse_select_stmt(tokens, i)?;
        if !matches!(tokens.get(*i), Some(Token::RParen)) {
            return Err("CTE missing closing ')'".into());
        }
        *i += 1;
        out.push(Cte {
            name,
            query: Box::new(query),
        });
        if matches!(tokens.get(*i), Some(Token::Comma)) {
            *i += 1;
            continue;
        }
        break;
    }
    Ok(out)
}

fn parse_qualified_table(
    tokens: &[Token],
    i: &mut usize,
) -> Result<(Option<String>, String), String> {
    let first = ident_at(tokens, *i)
        .ok_or("expected table name")?
        .to_lowercase();
    *i += 1;
    if matches!(tokens.get(*i), Some(Token::Dot)) {
        *i += 1;
        let table = ident_at(tokens, *i)
            .ok_or("expected table after namespace.")?
            .to_lowercase();
        *i += 1;
        Ok((Some(first), table))
    } else {
        Ok((None, first))
    }
}

fn parse_order_by(tokens: &[Token], i: &mut usize) -> Result<OrderBy, String> {
    expect_ident(tokens, i, "ORDER")?;
    expect_ident(tokens, i, "BY")?;
    let column = ident_at(tokens, *i)
        .ok_or("ORDER BY requires a column name or SCORE")?
        .to_lowercase();
    *i += 1;
    let default_desc = column == "score";
    let descending = match ident_at(tokens, *i).as_deref() {
        Some("ASC") => {
            *i += 1;
            false
        }
        Some("DESC") => {
            *i += 1;
            true
        }
        _ => default_desc,
    };
    Ok(OrderBy { column, descending })
}

pub fn parse(input: &str) -> Result<Vec<Stmt>, String> {
    let tokens = tokenize(input);
    let mut i = 0;
    let mut out = Vec::new();
    while !matches!(tokens.get(i), Some(Token::Eof) | None) {
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "CREATE") {
            if matches!(tokens.get(i + 1), Some(Token::Ident(k)) if k == "MATERIALIZED")
                && matches!(tokens.get(i + 2), Some(Token::Ident(k)) if k == "VIEW")
            {
                i += 3;
                let name = ident_at(&tokens, i)
                    .ok_or("materialized view name")?
                    .to_lowercase();
                i += 1;
                expect_ident(&tokens, &mut i, "AS")?;
                let select = parse_select_stmt(&tokens, &mut i)?;
                out.push(Stmt::CreateMaterializedView(CreateMaterializedViewStmt {
                    name,
                    select,
                }));
                continue;
            }
            if matches!(tokens.get(i + 1), Some(Token::Ident(k)) if k == "TABLE") {
                i += 2;
                let (namespace, name) = parse_qualified_table(&tokens, &mut i)?;
                let mut mode = "HYBRID".into();
                let columns = parse_column_defs(&tokens, &mut i)?;
                if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "USING") {
                    i += 2;
                    if let Some(Token::Ident(m)) = tokens.get(i - 1) {
                        mode = m.clone();
                    }
                }
                out.push(Stmt::CreateTable(CreateTableStmt {
                    namespace,
                    name,
                    mode,
                    columns,
                }));
                continue;
            }
            if matches!(tokens.get(i + 1), Some(Token::Ident(k)) if k == "INDEX") {
                i += 2;
                let name = match tokens.get(i) {
                    Some(Token::Ident(n)) => {
                        i += 1;
                        n.clone()
                    }
                    _ => return Err("index name after CREATE INDEX".into()),
                };
                if !matches!(tokens.get(i), Some(Token::Ident(k)) if k == "ON") {
                    return Err("CREATE INDEX requires ON table".into());
                }
                i += 1;
                let (_ns, table) = parse_qualified_table(&tokens, &mut i)?;
                if !matches!(tokens.get(i), Some(Token::LParen)) {
                    return Err("CREATE INDEX requires (column)".into());
                }
                i += 1;
                let column = match tokens.get(i) {
                    Some(Token::Ident(n)) => {
                        i += 1;
                        n.clone()
                    }
                    _ => return Err("column name in CREATE INDEX".into()),
                };
                if !matches!(tokens.get(i), Some(Token::RParen)) {
                    return Err("CREATE INDEX missing closing paren".into());
                }
                i += 1;
                let mut using = "HNSW".into();
                if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "USING") {
                    i += 1;
                    if let Some(Token::Ident(m)) = tokens.get(i) {
                        using = m.clone();
                        i += 1;
                    }
                }
                out.push(Stmt::CreateIndex(CreateIndexStmt {
                    name,
                    namespace: None,
                    table,
                    column,
                    using,
                }));
                continue;
            }
        }
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "COMPACT") {
            i += 1;
            expect_ident(&tokens, &mut i, "TABLE")?;
            let table = ident_at(&tokens, i)
                .ok_or("table name after COMPACT TABLE")?
                .to_lowercase();
            i += 1;
            let full = matches!(tokens.get(i), Some(Token::Ident(k)) if k == "FULL");
            if full {
                i += 1;
            }
            out.push(Stmt::CompactTable { table, full });
            continue;
        }
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "ALTER") {
            i += 1;
            expect_ident(&tokens, &mut i, "TABLE")?;
            let table = ident_at(&tokens, i)
                .ok_or("table name after ALTER TABLE")?
                .to_lowercase();
            i += 1;
            if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "ALTER") {
                i += 1;
                expect_ident(&tokens, &mut i, "COLUMN")?;
                let column = ident_at(&tokens, i)
                    .ok_or("column name after ALTER COLUMN")?
                    .to_lowercase();
                i += 1;
                expect_ident(&tokens, &mut i, "TYPE")?;
                let column_type = parse_type_at(&tokens, &mut i)?;
                let rewrite = matches!(tokens.get(i), Some(Token::Ident(k)) if k == "REWRITE");
                if rewrite {
                    i += 1;
                }
                out.push(Stmt::AlterTableAlterColumnType {
                    table,
                    column,
                    column_type,
                    rewrite,
                });
                continue;
            }
            expect_ident(&tokens, &mut i, "SET")?;
            expect_ident(&tokens, &mut i, "SEGMENT_WORKERS")?;
            if !matches!(tokens.get(i), Some(Token::Eq)) {
                return Err("ALTER TABLE SET SEGMENT_WORKERS requires =".into());
            }
            i += 1;
            let workers = match tokens.get(i) {
                Some(Token::Number(n)) => {
                    i += 1;
                    *n
                }
                _ => return Err("segment worker count after =".into()),
            };
            out.push(Stmt::AlterTableSetSegmentWorkers { table, workers });
            continue;
        }
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "DELETE") {
            i += 1;
            expect_ident(&tokens, &mut i, "FROM")?;
            let (_ns, table) = parse_qualified_table(&tokens, &mut i)?;
            let where_clause = if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "WHERE") {
                Some(parse_where_clause(&tokens, &mut i)?)
            } else {
                None
            };
            out.push(Stmt::Delete {
                table,
                where_clause,
            });
            continue;
        }
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "DROP") {
            if matches!(tokens.get(i + 1), Some(Token::Ident(k)) if k == "MATERIALIZED")
                && matches!(tokens.get(i + 2), Some(Token::Ident(k)) if k == "VIEW")
            {
                i += 3;
                let name = ident_at(&tokens, i)
                    .ok_or("materialized view name after DROP MATERIALIZED VIEW")?
                    .to_lowercase();
                i += 1;
                out.push(Stmt::DropMaterializedView { name });
                continue;
            }
            if matches!(tokens.get(i + 1), Some(Token::Ident(k)) if k == "TABLE") {
                i += 2;
                let name = match tokens.get(i) {
                    Some(Token::Ident(n)) => {
                        i += 1;
                        n.to_lowercase()
                    }
                    _ => return Err("table name after DROP TABLE".into()),
                };
                out.push(Stmt::DropTable { name });
                continue;
            }
        }
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "SHOW") {
            i += 1;
            match tokens.get(i) {
                Some(Token::Ident(k)) if k == "TABLES" => {
                    i += 1;
                    out.push(Stmt::ShowTables);
                }
                Some(Token::Ident(k)) if k == "MATERIALIZED" => {
                    i += 1;
                    expect_ident(&tokens, &mut i, "VIEWS")?;
                    out.push(Stmt::ShowMaterializedViews);
                }
                Some(Token::Ident(k)) if k == "INDEXES" || k == "INDEX" => {
                    i += 1;
                    expect_ident(&tokens, &mut i, "FROM")?;
                    let (_ns, table) = parse_qualified_table(&tokens, &mut i)?;
                    out.push(Stmt::ShowIndexes { table });
                }
                Some(Token::Ident(k)) if k == "CREATE" => {
                    i += 1;
                    expect_ident(&tokens, &mut i, "TABLE")?;
                    let (_ns, table) = parse_qualified_table(&tokens, &mut i)?;
                    out.push(Stmt::ShowCreateTable { table });
                }
                _ => {
                    return Err(
                        "expected TABLES, INDEXES, CREATE TABLE, or MATERIALIZED VIEWS after SHOW"
                            .into(),
                    );
                }
            }
            continue;
        }
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "REFRESH") {
            i += 1;
            expect_ident(&tokens, &mut i, "MATERIALIZED")?;
            expect_ident(&tokens, &mut i, "VIEW")?;
            let name = ident_at(&tokens, i)
                .ok_or("materialized view name after REFRESH")?
                .to_lowercase();
            i += 1;
            out.push(Stmt::RefreshMaterializedView { name });
            continue;
        }
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "DESCRIBE" || k == "DESC") {
            i += 1;
            let (_ns, name) = parse_qualified_table(&tokens, &mut i)?;
            out.push(Stmt::Describe { name });
            continue;
        }
        let stream_prefix = matches!(tokens.get(i), Some(Token::Ident(k)) if k == "STREAM") && {
            i += 1;
            true
        };
        let explain_prefix = matches!(tokens.get(i), Some(Token::Ident(k)) if k == "EXPLAIN") && {
            i += 1;
            true
        };
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "WITH") {
            let ctes = parse_ctes(&tokens, &mut i)?;
            if !matches!(tokens.get(i), Some(Token::Ident(k)) if k == "SELECT") {
                return Err("WITH requires a trailing SELECT".into());
            }
            let mut select = parse_select_stmt(&tokens, &mut i)?;
            select.ctes = ctes;
            select.stream |= stream_prefix;
            select.explain |= explain_prefix;
            out.push(Stmt::Select(select));
            continue;
        }
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "SELECT") {
            let mut select = parse_select_stmt(&tokens, &mut i)?;
            select.stream |= stream_prefix;
            select.explain |= explain_prefix;
            out.push(Stmt::Select(select));
            continue;
        }
        if stream_prefix {
            return Err("STREAM requires a SELECT statement".into());
        }
        if explain_prefix {
            return Err("EXPLAIN requires a SELECT statement".into());
        }
        i += 1;
    }
    Ok(out)
}
