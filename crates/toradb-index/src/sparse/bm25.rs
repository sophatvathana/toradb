use std::collections::HashMap;

use serde::{Deserialize, Serialize};
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
    pub fn doc_count(&self) -> u32 {
        self.num_docs
    }

    pub fn add_document(&mut self, id: DocId, text: &str) {
        self.num_docs += 1;
        let mut tf_map: HashMap<String, u32> = HashMap::new();
        let mut len = 0u32;
        let mut terms_buf = Vec::new();
        tokenize_into(text, &mut terms_buf);
        for term in terms_buf {
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

    pub fn snapshot(&self) -> Bm25Snapshot {
        Bm25Snapshot {
            postings: self.postings.clone(),
            doc_len: self.doc_len.clone(),
            doc_freq: self.doc_freq.clone(),
            num_docs: self.num_docs,
            avg_dl: self.avg_dl,
        }
    }

    pub fn from_snapshot(snap: Bm25Snapshot) -> Self {
        Self {
            postings: snap.postings,
            doc_len: snap.doc_len,
            doc_freq: snap.doc_freq,
            num_docs: snap.num_docs,
            avg_dl: snap.avg_dl,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Bm25Snapshot {
    pub postings: HashMap<String, Vec<(DocId, u32)>>,
    pub doc_len: HashMap<DocId, u32>,
    pub doc_freq: HashMap<String, u32>,
    pub num_docs: u32,
    pub avg_dl: f32,
}

impl Bm25Snapshot {
    /// Build a snapshot for a disjoint document set (e.g. one Parquet segment).
    pub fn from_documents(docs: impl IntoIterator<Item = (DocId, impl AsRef<str>)>) -> Self {
        let mut index = Bm25Index::default();
        for (id, text) in docs {
            index.add_document(id, text.as_ref());
        }
        index.snapshot()
    }

    /// Merge another snapshot whose document ids do not overlap with this one.
    pub fn merge(&mut self, other: Bm25Snapshot) {
        for (term, mut posts) in other.postings {
            self.postings.entry(term).or_default().append(&mut posts);
        }
        self.doc_len.extend(other.doc_len);
        for (term, df) in other.doc_freq {
            *self.doc_freq.entry(term).or_default() += df;
        }
        self.num_docs += other.num_docs;
        let total_len: u32 = self.doc_len.values().sum();
        self.avg_dl = if self.num_docs > 0 {
            total_len as f32 / self.num_docs as f32
        } else {
            0.0
        };
    }

    /// Balanced tree merge of disjoint segment snapshots
    pub fn merge_snapshots_tree(mut snaps: Vec<Bm25Snapshot>) -> Option<Bm25Snapshot> {
        if snaps.is_empty() {
            return None;
        }
        while snaps.len() > 1 {
            let mut next = Vec::new();
            let mut i = 0;
            while i < snaps.len() {
                if i + 1 < snaps.len() {
                    let mut left = snaps[i].clone();
                    left.merge(snaps[i + 1].clone());
                    next.push(left);
                    i += 2;
                } else {
                    next.push(snaps[i].clone());
                    i += 1;
                }
            }
            snaps = next;
        }
        snaps.pop()
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

#[inline]
fn fold_char(c: char) -> char {
    if c.is_ascii_uppercase() {
        c.to_ascii_lowercase()
    } else {
        c
    }
}

/// Tokenize for BM25 without allocating a lowercased copy of the full text.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    tokenize_into(text, &mut tokens);
    tokens
}

/// Append tokens from `text` into `out` (reuses allocation in `out` only).
pub fn tokenize_into(text: &str, out: &mut Vec<String>) {
    let mut buf = String::new();
    let mut active: Option<Script> = None;

    let flush = |buf: &mut String, out: &mut Vec<String>| {
        if !buf.is_empty() {
            out.push(buf.clone());
            buf.clear();
        }
    };

    for c in text.chars() {
        let c = fold_char(c);
        match script_of(c) {
            Some(script) => {
                if active == Some(script) || active.is_none() {
                    active = Some(script);
                    buf.push(c);
                } else {
                    flush(&mut buf, out);
                    active = Some(script);
                    buf.push(c);
                }
            }
            None if c.is_whitespace() => {
                flush(&mut buf, out);
                active = None;
            }
            None => {
                flush(&mut buf, out);
                active = None;
            }
        }
    }
    flush(&mut buf, out);
}

#[cfg(test)]
mod tests {
    use super::{tokenize, Bm25Index, Bm25Snapshot};

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

    #[test]
    fn binary_codec_roundtrip() {
        let snap = Bm25Snapshot::from_documents([(0u64, "alpha beta gamma")]);
        let bytes = crate::sparse::bm25_codec::encode_snapshot(&snap).unwrap();
        let back = crate::sparse::bm25_codec::decode_snapshot(&bytes).unwrap();
        let index = Bm25Index::from_snapshot(back);
        assert!(!index.search("gamma", 1).is_empty());
    }

    #[test]
    fn merge_snapshots_combines_disjoint_docs() {
        let a = Bm25Snapshot::from_documents([(0u64, "alpha beta")]);
        let b = Bm25Snapshot::from_documents([(1u64, "gamma delta")]);
        let mut merged = a;
        merged.merge(b);
        let index = Bm25Index::from_snapshot(merged);
        assert!(!index.search("alpha", 1).is_empty());
        assert!(!index.search("gamma", 1).is_empty());
    }
}
