use crate::ast::*;
use crate::lexer::{tokenize, Token};

pub fn parse(input: &str) -> Result<Vec<Stmt>, String> {
    let tokens = tokenize(input);
    let mut i = 0;
    let mut out = Vec::new();
    while !matches!(tokens.get(i), Some(Token::Eof) | None) {
        if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "CREATE") {
            if matches!(tokens.get(i + 1), Some(Token::Ident(k)) if k == "TABLE") {
                i += 2;
                let name = match tokens.get(i) { Some(Token::Ident(n)) => { i += 1; n.clone() }, _ => return Err("table name".into()) };
                let mut mode = "HYBRID".into();
                let mut columns = vec![];
                if matches!(tokens.get(i), Some(Token::Ident(k)) if k == "USING") {
                    i += 2;
                    if let Some(Token::Ident(m)) = tokens.get(i - 1) { mode = m.clone(); }
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
            let table = match tokens.get(i) { Some(Token::Ident(n)) => { i += 1; n.to_lowercase() }, _ => return Err("from table".into()) };
            let mut sparse = None;
            let mut vector = false;
            let mut limit = 20;
            let mut group_by = None;
            while i < tokens.len() {
                match tokens.get(i) {
                    Some(Token::Ident(k)) if k == "FROM" => { i += 1; }
                    Some(Token::Ident(k)) if k == "SPARSE" => { i += 3; sparse = Some("bm25".into()); }
                    Some(Token::Ident(k)) if k == "VECTOR" => { vector = true; i += 1; }
                    Some(Token::Ident(k)) if k == "LIMIT" => { i += 1; if let Some(Token::Number(n)) = tokens.get(i) { limit = *n; i += 1; } }
                    Some(Token::Ident(k)) if k == "GROUP" => { i += 2; if let Some(Token::Ident(g)) = tokens.get(i) { group_by = Some(g.to_lowercase()); i += 1; } }
                    Some(Token::Eof) | None => break,
                    _ => i += 1,
                }
            }
            out.push(Stmt::Select(SelectStmt { table, sparse, vector, limit, group_by }));
            continue;
        }
        i += 1;
    }
    Ok(out)
}
