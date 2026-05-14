#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    Number(u32),
    String(String),
    LParen,
    RParen,
    Comma,
    Semi,
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
    Eof,
}

pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'\'' {
            i += 1;
            let start = i;
            let mut s = String::new();
            while i < bytes.len() {
                if bytes[i] == b'\'' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        s.push('\'');
                        i += 2;
                        continue;
                    }
                    break;
                }
                s.push(bytes[i] as char);
                i += 1;
            }
            if i >= bytes.len() || bytes[i] != b'\'' {
                s = String::from_utf8_lossy(&bytes[start..i]).into_owned();
            } else {
                i += 1;
            }
            tokens.push(Token::String(s));
            continue;
        }
        if c.is_ascii_alphanumeric() || c == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &input[start..i];
            if let Ok(n) = word.parse::<u32>() {
                tokens.push(Token::Number(n));
            } else {
                tokens.push(Token::Ident(word.to_uppercase()));
            }
            continue;
        }
        match c {
            b'(' => tokens.push(Token::LParen),
            b')' => tokens.push(Token::RParen),
            b',' => tokens.push(Token::Comma),
            b';' => tokens.push(Token::Semi),
            b'!' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                tokens.push(Token::Ne);
                i += 2;
                continue;
            }
            b'<' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                tokens.push(Token::Lte);
                i += 2;
                continue;
            }
            b'>' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => {
                tokens.push(Token::Gte);
                i += 2;
                continue;
            }
            b'<' => tokens.push(Token::Lt),
            b'>' => tokens.push(Token::Gt),
            b'=' => tokens.push(Token::Eq),
            _ => {}
        }
        i += 1;
    }
    tokens.push(Token::Eof);
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_sparse_query() {
        let t = tokenize("BM25('Nikola Tesla motor')");
        assert!(t.iter().any(|tok| matches!(tok, Token::String(s) if s.contains("Nikola"))));
    }
}
