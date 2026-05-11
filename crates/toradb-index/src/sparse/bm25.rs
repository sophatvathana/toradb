use std::collections::HashMap;

use toradb_core::{CandidateSet, DocId};

const K1: f32 = 1.2;
const B: f32 = 0.75;

#[derive(Debug, Default)]
pub struct Bm25Index {
    postings: HashMap<String, Vec<(DocId, u32)>>,
    doc_len: HashMap<DocId, u32>,
    doc_freq: HashMap<String, u32>,
    num_docs: u32,
    avg_dl: f32,
}

impl Bm25Index {
    pub fn add_document(&mut self, id: DocId, text: &str) {
        self.num_docs += 1;
        let mut tf_map: HashMap<String, u32> = HashMap::new();
        let mut len = 0u32;
        for term in tokenize(text) {
            len += 1;
            *tf_map.entry(term).or_default() += 1;
        }
        self.doc_len.insert(id, len);
        let total_len: u32 = self.doc_len.values().sum();
        self.avg_dl = if self.num_docs > 0 {
            total_len as f32 / self.num_docs as f32
        } else {
            0.0
        };
        for (term, tf) in tf_map {
            *self.doc_freq.entry(term.clone()).or_default() += 1;
            self.postings.entry(term).or_default().push((id, tf));
        }
    }

    pub fn search(&self, query: &str, k: usize) -> CandidateSet {
        let mut scores: HashMap<DocId, f32> = HashMap::new();
        let n = self.num_docs.max(1) as f32;
        for term in tokenize(query) {
            let Some(postings) = self.postings.get(&term) else {
                continue;
            };
            let df = *self.doc_freq.get(&term).unwrap_or(&0) as f32;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            for &(doc_id, tf) in postings {
                let dl = *self.doc_len.get(&doc_id).unwrap_or(&0) as f32;
                let tf = tf as f32;
                let denom = tf + K1 * (1.0 - B + B * dl / self.avg_dl.max(1.0));
                let score = idf * (tf * (K1 + 1.0)) / denom;
                *scores.entry(doc_id).or_default() += score;
            }
        }
        let mut ranked: Vec<(DocId, f32)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(k);
        let mut out = CandidateSet::with_capacity(ranked.len());
        for (id, score) in ranked {
            out.push(id, score);
        }
        out
    }
}

fn is_khmer(c: char) -> bool {
    ('\u{1780}'..='\u{17ff}').contains(&c) || ('\u{19e0}'..='\u{19ff}').contains(&c)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Khmer,
    Latin,
}

fn script_of(c: char) -> Option<Script> {
    if c.is_ascii_alphanumeric() {
        Some(Script::Latin)
    } else if is_khmer(c) {
        Some(Script::Khmer)
    } else {
        None
    }
}

/// Tokenize for BM25. Khmer vowel marks are not alphanumeric in Unicode, so we group
/// contiguous Khmer script into terms and split Latin words separately.
pub fn tokenize(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut tokens = Vec::new();
    let mut buf = String::new();
    let mut active: Option<Script> = None;

    let flush = |buf: &mut String, tokens: &mut Vec<String>| {
        if !buf.is_empty() {
            tokens.push(buf.clone());
            buf.clear();
        }
    };

    for c in lower.chars() {
        match script_of(c) {
            Some(script) => {
                if active == Some(script) || active.is_none() {
                    active = Some(script);
                    buf.push(c);
                } else {
                    flush(&mut buf, &mut tokens);
                    active = Some(script);
                    buf.push(c);
                }
            }
            None if c.is_whitespace() => {
                flush(&mut buf, &mut tokens);
                active = None;
            }
            None => {
                flush(&mut buf, &mut tokens);
                active = None;
            }
        }
    }
    flush(&mut buf, &mut tokens);
    tokens
}

#[cfg(test)]
mod tests {
    use super::tokenize;

    #[test]
    fn tokenize_english_tesla_terms() {
        let terms = tokenize("Nikola Tesla alternating current motor");
        assert!(terms.contains(&"nikola".to_string()));
        assert!(terms.contains(&"tesla".to_string()));
        assert!(terms.contains(&"alternating".to_string()));
    }

    #[test]
    fn khmer_terms_stay_whole() {
        let terms = tokenize("ឯកសារ អំពី ភាសា");
        assert!(terms.contains(&"ឯកសារ".to_string()));
    }
}
