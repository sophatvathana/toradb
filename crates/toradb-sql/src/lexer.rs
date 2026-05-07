#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    Number(u32),
    LParen,
    RParen,
    Comma,
    Semi,
    Eof,
}

pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut it = input.split_whitespace().peekable();
    while let Some(word) = it.next() {
        let w = word.trim_matches(|c: char| c == ',' || c == ';' || c == '(' || c == ')');
        if w.is_empty() { continue; }
        if w.chars().all(|c| c.is_ascii_digit()) {
            tokens.push(Token::Number(w.parse().unwrap_or(0)));
        } else {
            tokens.push(Token::Ident(w.to_uppercase()));
        }
    }
    tokens.push(Token::Eof);
    tokens
}
