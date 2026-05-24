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

fn parse_where_clause(tokens: &[Token], i: &mut usize) -> Result<WherePred, String> {
    expect_ident(tokens, i, "WHERE")?;
    let column = ident_at(tokens, *i).ok_or("WHERE requires column name")?.to_lowercase();
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

    let op = tokens
        .get(*i)
        .and_then(parse_compare_op)
        .ok_or("WHERE requires comparison operator")?;
    *i += 1;
    let value = parse_literal(tokens, i)?;
    Ok(WherePred::Compare { column, op, value })
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

fn parse_sparse_search(tokens: &[Token], i: &mut usize) -> Result<(Option<String>, Option<String>), String> {
    // SPARSE SEARCH <col> BM25 ( 'query' )
    expect_ident(tokens, i, "SPARSE")?;
    expect_ident(tokens, i, "SEARCH")?;
    if ident_at(tokens, *i).is_some() {
        *i += 1; // column
    }
    let method = ident_at(tokens, *i).map(|m| m.to_lowercase());
    if method.is_some() {
        *i += 1;
    }
    let mut query = None;
    if matches!(tokens.get(*i), Some(Token::LParen)) {
        *i += 1;
        if let Some(Token::String(q)) = tokens.get(*i) {
            query = Some(q.clone());
            *i += 1;
        }
        if matches!(tokens.get(*i), Some(Token::RParen)) {
            *i += 1;
        }
    }
    Ok((method, query))
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
    let left_table = ident_at(tokens, *i)
        .ok_or("table name after FROM")?
        .to_lowercase();
    *i += 1;
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
    let mut order_by_score_desc = None;
    let mut distributed = false;
    let mut stream = false;
    let mut group_by = None;
    let mut where_clause = None;
    while *i < tokens.len() && !matches!(tokens.get(*i), Some(Token::Eof)) {
        match tokens.get(*i) {
            Some(Token::Ident(k)) if k == "DISTRIBUTED" => {
                distributed = true;
                *i += 1;
            }
            Some(Token::Ident(k)) if k == "STREAM" => {
                stream = true;
                *i += 1;
            }
            Some(Token::Ident(k)) if k == "SPARSE" => {
                let (method, query) = parse_sparse_search(tokens, i)?;
                sparse = method.or(Some("bm25".into()));
                sparse_query = query;
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
                order_by_score_desc = Some(parse_order_by_score(tokens, i)?);
            }
            Some(Token::Ident(k)) if k == "GROUP" => {
                *i += 2;
                if let Some(Token::Ident(g)) = tokens.get(*i) {
                    group_by = Some(g.to_lowercase());
                    *i += 1;
                }
            }
            Some(Token::Ident(k)) if k == "WHERE" => {
                where_clause = Some(parse_where_clause(tokens, i)?);
            }
            Some(Token::Semi) | Some(Token::Eof) => break,
            _ => *i += 1,
        }
    }
    Ok(SelectStmt {
        table,
        join,
        select_items,
        sparse,
        sparse_query,
        vector,
        vector_query,
        vector_text,
        limit,
        offset,
        order_by_score_desc,
        distributed,
        stream,
        group_by,
        where_clause,
    })
}

fn parse_order_by_score(tokens: &[Token], i: &mut usize) -> Result<bool, String> {
    expect_ident(tokens, i, "ORDER")?;
    expect_ident(tokens, i, "BY")?;
    expect_ident(tokens, i, "SCORE")?;
    match ident_at(tokens, *i).as_deref() {
        Some("ASC") => {
            *i += 1;
            Ok(false)
        }
        Some("DESC") => {
            *i += 1;
            Ok(true)
        }
        _ => Ok(true),
    }
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
                let name = match tokens.get(i) {
                    Some(Token::Ident(n)) => {
                        i += 1;
                        n.clone()
                    }
                    _ => return Err("table name".into()),
                };
                let mut mode = "HYBRID".into();
                let columns = vec![];
                if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "USING") {
                    i += 2;
                    if let Some(Token::Ident(m)) = tokens.get(i - 1) {
                        mode = m.clone();
                    }
                }
                out.push(Stmt::CreateTable(CreateTableStmt { name, mode, columns }));
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
                let table = match tokens.get(i) {
                    Some(Token::Ident(n)) => {
                        i += 1;
                        n.to_lowercase()
                    }
                    _ => return Err("table name after ON".into()),
                };
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
                    table,
                    column,
                    using,
                }));
                continue;
            }
        }
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "ALTER") {
            i += 1;
            expect_ident(&tokens, &mut i, "TABLE")?;
            let table = ident_at(&tokens, i)
                .ok_or("table name after ALTER TABLE")?
                .to_lowercase();
            i += 1;
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
            i += 2;
            out.push(Stmt::ShowTables);
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
            let name = match tokens.get(i) {
                Some(Token::Ident(n)) => {
                    i += 1;
                    n.to_lowercase()
                }
                _ => return Err("table name after DESCRIBE".into()),
            };
            out.push(Stmt::Describe { name });
            continue;
        }
        let stream_prefix =
            matches!(tokens.get(i), Some(Token::Ident(k)) if k == "STREAM") && {
                i += 1;
                true
            };
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "SELECT") {
            let mut select = parse_select_stmt(&tokens, &mut i)?;
            select.stream |= stream_prefix;
            out.push(Stmt::Select(select));
            continue;
        }
        if stream_prefix {
            return Err("STREAM requires a SELECT statement".into());
        }
        i += 1;
    }
    Ok(out)
}
