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

fn parse_select_exprs(tokens: &[Token], i: &mut usize) -> Result<Vec<SelectExpr>, String> {
    let mut items = Vec::new();
    loop {
        if matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "FROM") {
            break;
        }
        if matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "COUNT") {
            *i += 1;
            if matches!(tokens.get(*i), Some(Token::LParen)) {
                *i += 1;
                if matches!(tokens.get(*i), Some(Token::Ident(k)) if k == "*") {
                    *i += 1;
                } else if ident_at(tokens, *i).is_some() {
                    *i += 1;
                }
                if matches!(tokens.get(*i), Some(Token::RParen)) {
                    *i += 1;
                }
            }
            items.push(SelectExpr::CountStar);
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

fn parse_where_eq(tokens: &[Token], i: &mut usize) -> Result<WhereEq, String> {
    expect_ident(tokens, i, "WHERE")?;
    let column = ident_at(tokens, *i).ok_or("WHERE requires column name")?;
    *i += 1;
    if !matches!(tokens.get(*i), Some(Token::Eq)) {
        return Err("WHERE requires =".into());
    }
    *i += 1;
    let value = match tokens.get(*i) {
        Some(Token::String(s)) => {
            *i += 1;
            s.clone()
        }
        Some(Token::Ident(s)) => {
            *i += 1;
            s.to_lowercase()
        }
        _ => return Err("WHERE requires string or identifier value".into()),
    };
    Ok(WhereEq {
        column: column.to_lowercase(),
        value,
    })
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

pub fn parse(input: &str) -> Result<Vec<Stmt>, String> {
    let tokens = tokenize(input);
    let mut i = 0;
    let mut out = Vec::new();
    while !matches!(tokens.get(i), Some(Token::Eof) | None) {
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "CREATE") {
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
        }
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "SHOW") {
            i += 2;
            out.push(Stmt::ShowTables);
            continue;
        }
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "SELECT") {
            i += 1;
            let select_items = parse_select_exprs(&tokens, &mut i)?;
            if !matches!(tokens.get(i), Some(Token::Ident(k)) if k == "FROM") {
                return Err("SELECT requires FROM".into());
            }
            i += 1;
            let table = match tokens.get(i) {
                Some(Token::Ident(n)) => {
                    i += 1;
                    n.to_lowercase()
                }
                _ => return Err("table name after FROM".into()),
            };
            let mut sparse = None;
            let mut sparse_query = None;
            let mut vector = false;
            let mut limit = 20;
            let mut group_by = None;
            let mut where_eq = None;
            while i < tokens.len() && !matches!(tokens.get(i), Some(Token::Eof)) {
                match tokens.get(i) {
                    Some(Token::Ident(k)) if k == "SPARSE" => {
                        let (method, query) = parse_sparse_search(&tokens, &mut i)?;
                        sparse = method.or(Some("bm25".into()));
                        sparse_query = query;
                    }
                    Some(Token::Ident(k)) if k == "VECTOR" => {
                        vector = true;
                        if matches!(tokens.get(i + 1), Some(Token::Ident(k)) if k == "SEARCH") {
                            i += 2;
                            if ident_at(&tokens, i).is_some() {
                                i += 1;
                            }
                        } else {
                            i += 1;
                        }
                    }
                    Some(Token::Ident(k)) if k == "LIMIT" => {
                        i += 1;
                        if let Some(Token::Number(n)) = tokens.get(i) {
                            limit = *n;
                            i += 1;
                        }
                    }
                    Some(Token::Ident(k)) if k == "GROUP" => {
                        i += 2;
                        if let Some(Token::Ident(g)) = tokens.get(i) {
                            group_by = Some(g.to_lowercase());
                            i += 1;
                        }
                    }
                    Some(Token::Ident(k)) if k == "WHERE" => {
                        where_eq = Some(parse_where_eq(&tokens, &mut i)?);
                    }
                    Some(Token::Semi) | Some(Token::Eof) => break,
                    _ => i += 1,
                }
            }
            out.push(Stmt::Select(SelectStmt {
                table,
                select_items,
                sparse,
                sparse_query,
                vector,
                limit,
                group_by,
                where_eq,
            }));
            continue;
        }
        i += 1;
    }
    Ok(out)
}
