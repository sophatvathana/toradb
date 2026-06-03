//! Search highlighting / snippets.
use std::collections::HashSet;

use toradb_index::sparse::bm25::tokenize;

pub fn snippet_query_tokens(query: &str) -> HashSet<String> {
    tokenize(query).into_iter().collect()
}

#[inline]
fn is_khmer(c: char) -> bool {
    ('\u{1780}'..='\u{17ff}').contains(&c) || ('\u{19e0}'..='\u{19ff}').contains(&c)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Khmer,
    Latin,
}

#[inline]
fn script_of(c: char) -> Option<Script> {
    if c.is_ascii_alphanumeric() {
        Some(Script::Latin)
    } else if is_khmer(c) {
        Some(Script::Khmer)
    } else {
        None
    }
}

#[inline]
fn fold_char(c: char) -> char {
    if c.is_ascii_uppercase() {
        c.to_ascii_lowercase()
    } else {
        c
    }
}

struct TokenSpan {
    start: usize,
    end: usize,
    matched: bool,
}

fn scan_tokens(text: &str, query_tokens: &HashSet<String>) -> Vec<TokenSpan> {
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut active: Option<Script> = None;
    let mut tok_start = 0usize;

    let flush = |buf: &mut String, start: usize, end: usize, spans: &mut Vec<TokenSpan>| {
        if !buf.is_empty() {
            let matched = query_tokens.contains(buf.as_str());
            spans.push(TokenSpan {
                start,
                end,
                matched,
            });
            buf.clear();
        }
    };

    let mut last_end = 0usize;
    for (byte_idx, ch) in text.char_indices() {
        let c = fold_char(ch);
        let char_end = byte_idx + ch.len_utf8();
        match script_of(c) {
            Some(script) => {
                if active.is_none() {
                    tok_start = byte_idx;
                    active = Some(script);
                    buf.push(c);
                } else if active == Some(script) {
                    buf.push(c);
                } else {
                    flush(&mut buf, tok_start, last_end, &mut spans);
                    tok_start = byte_idx;
                    active = Some(script);
                    buf.push(c);
                }
            }
            None => {
                flush(&mut buf, tok_start, last_end, &mut spans);
                active = None;
            }
        }
        last_end = char_end;
    }
    flush(&mut buf, tok_start, last_end, &mut spans);
    spans
}

fn floor_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn generate_snippet(
    text: &str,
    query_tokens: &HashSet<String>,
    max_chars: usize,
    open: &str,
    close: &str,
) -> String {
    let max_chars = max_chars.max(16);
    if text.is_empty() {
        return String::new();
    }
    let spans = scan_tokens(text, query_tokens);
    let match_spans: Vec<&TokenSpan> = spans.iter().filter(|s| s.matched).collect();

    if match_spans.is_empty() {
        let end = ceil_boundary(text, max_chars.min(text.len()));
        let mut out = normalize_ws(&text[..end]);
        if end < text.len() {
            out.push('…');
        }
        return out;
    }

    let approx_bytes = max_chars;
    let mut best_center = match_spans[0].start;
    let mut best_count = 0usize;
    for anchor in &match_spans {
        let win_start = anchor.start.saturating_sub(approx_bytes / 3);
        let win_end = win_start + approx_bytes;
        let count = match_spans
            .iter()
            .filter(|s| s.start >= win_start && s.end <= win_end)
            .count();
        if count > best_count {
            best_count = count;
            best_center = anchor.start;
        }
    }

    let half = approx_bytes / 2;
    let raw_start = best_center.saturating_sub(half);
    let raw_end = (best_center + half).min(text.len());
    let start = floor_boundary(text, raw_start);
    let end = ceil_boundary(text, raw_end);

    let mut out = String::new();
    let mut cursor = start;
    for s in spans
        .iter()
        .filter(|s| s.matched && s.start >= start && s.end <= end)
    {
        if s.start < cursor {
            continue;
        }
        out.push_str(&text[cursor..s.start]);
        out.push_str(open);
        out.push_str(&text[s.start..s.end]);
        out.push_str(close);
        cursor = s.end;
    }
    out.push_str(&text[cursor..end]);

    let mut snippet = normalize_ws(&out);
    if start > 0 {
        snippet = format!("…{snippet}");
    }
    if end < text.len() {
        snippet.push('…');
    }
    snippet
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(words: &[&str]) -> HashSet<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn marks_matched_term() {
        let s = generate_snippet(
            "the alternating current motor",
            &q(&["current"]),
            160,
            "<em>",
            "</em>",
        );
        assert!(s.contains("<em>current</em>"), "{s}");
    }

    #[test]
    fn case_insensitive_match() {
        let s = generate_snippet("Nikola Tesla motor", &q(&["tesla"]), 160, "<em>", "</em>");
        assert!(s.contains("<em>Tesla</em>"), "{s}");
    }

    #[test]
    fn multi_term_marks_all_in_window() {
        let s = generate_snippet(
            "alternating current induction motor design",
            &q(&["current", "motor"]),
            160,
            "<em>",
            "</em>",
        );
        assert!(s.contains("<em>current</em>"), "{s}");
        assert!(s.contains("<em>motor</em>"), "{s}");
    }

    #[test]
    fn no_match_returns_plain_prefix() {
        let s = generate_snippet("alpha beta gamma", &q(&["zeta"]), 160, "<em>", "</em>");
        assert!(!s.contains("<em>"));
        assert!(s.starts_with("alpha"));
    }

    #[test]
    fn respects_max_chars() {
        let text = "word ".repeat(200); // 1000 chars
        let s = generate_snippet(&text, &q(&["zeta"]), 50, "<em>", "</em>");
        // Window ~50 chars + ellipsis; well under the full text.
        assert!(s.len() <= 80, "snippet too long: {}", s.len());
        assert!(s.ends_with('…'));
    }

    #[test]
    fn multibyte_text_no_panic() {
        // Khmer + Latin mix; ensure char-boundary slicing never panics.
        let text = "ការស្វែងរក tesla និង current នៅ ToraDB";
        let s = generate_snippet(text, &q(&["tesla", "current"]), 80, "<em>", "</em>");
        assert!(
            s.contains("<em>tesla</em>") || s.contains("<em>current</em>"),
            "{s}"
        );
    }

    #[test]
    fn centers_window_on_match_deep_in_text() {
        let prefix = "filler ".repeat(50); // push the match far in
        let text = format!("{prefix}alternating current motor {prefix}");
        let s = generate_snippet(&text, &q(&["current"]), 80, "<em>", "</em>");
        assert!(s.contains("<em>current</em>"), "{s}");
        assert!(s.starts_with('…'), "should be truncated at the front: {s}");
    }
}
