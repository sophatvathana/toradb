use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use toradb_core::{CandidateSet, DocId};

pub(crate) const K1: f32 = 1.2;
pub(crate) const B: f32 = 0.75;

#[derive(Debug, Default)]
pub struct Bm25Index {
    postings: HashMap<String, Vec<(DocId, u32)>>,
    doc_len: HashMap<DocId, u32>,
    doc_freq: HashMap<String, u32>,
    num_docs: u32,
    total_doc_len: u64,
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
        self.total_doc_len += u64::from(len);
        self.avg_dl = self.total_doc_len as f32 / self.num_docs as f32;
        for (term, tf) in tf_map {
            *self.doc_freq.entry(term.clone()).or_default() += 1;
            self.postings.entry(term).or_default().push((id, tf));
        }
    }

    pub fn search(&self, query: &str, k: usize) -> CandidateSet {
        bm25_search(
            &self.postings,
            &self.doc_len,
            &self.doc_freq,
            self.num_docs,
            self.avg_dl,
            query,
            k,
        )
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
        let total_doc_len = snap.doc_len.values().map(|&l| u64::from(l)).sum();
        Self {
            postings: snap.postings,
            doc_len: snap.doc_len,
            doc_freq: snap.doc_freq,
            num_docs: snap.num_docs,
            total_doc_len,
            avg_dl: snap.avg_dl,
        }
    }
}

/// Incremental BM25 build (e.g. streaming one Parquet segment at a time).
#[derive(Debug, Default)]
pub struct Bm25Builder(Bm25Index);

impl Bm25Builder {
    pub fn add(&mut self, id: DocId, text: &str) {
        self.0.add_document(id, text);
    }

    pub fn finish(self) -> Bm25Snapshot {
        self.0.snapshot()
    }
}

#[derive(
    Debug, Clone, Serialize, Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct Bm25Snapshot {
    pub postings: HashMap<String, Vec<(DocId, u32)>>,
    pub doc_len: HashMap<DocId, u32>,
    pub doc_freq: HashMap<String, u32>,
    pub num_docs: u32,
    pub avg_dl: f32,
}

impl Bm25Snapshot {
    pub fn search(&self, query: &str, k: usize) -> CandidateSet {
        bm25_search(
            &self.postings,
            &self.doc_len,
            &self.doc_freq,
            self.num_docs,
            self.avg_dl,
            query,
            k,
        )
    }

    /// Build a snapshot for a disjoint document set (e.g. one Parquet segment).
    pub fn from_documents(docs: impl IntoIterator<Item = (DocId, impl AsRef<str>)>) -> Self {
        let mut builder = Bm25Builder::default();
        for (id, text) in docs {
            builder.add(id, text.as_ref());
        }
        builder.finish()
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
        let total_len: u64 = self.doc_len.values().map(|&l| u64::from(l)).sum();
        self.avg_dl = if self.num_docs > 0 {
            total_len as f32 / self.num_docs as f32
        } else {
            0.0
        };
    }

    /// Merge disjoint snapshots using a term interner (lower peak RAM than repeated `HashMap` merges).
    pub fn merge_snapshots_interned(snaps: Vec<Bm25Snapshot>) -> Option<Bm25Snapshot> {
        if snaps.is_empty() {
            return None;
        }
        if snaps.len() == 1 {
            return Some(snaps.into_iter().next().unwrap());
        }
        let mut interner = Bm25Interner::default();
        for snap in snaps {
            interner.absorb_snapshot(snap);
        }
        Some(interner.into_snapshot())
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

/// Interned term dictionary for faster multi-segment BM25 merges.
#[derive(Debug, Default)]
pub struct Bm25Interner {
    terms: Vec<String>,
    term_to_id: HashMap<String, u32>,
    postings: Vec<Vec<(DocId, u32)>>,
    doc_len: HashMap<DocId, u32>,
    doc_freq: Vec<u32>,
    num_docs: u32,
    total_doc_len: u64,
}

impl Bm25Interner {
    pub fn absorb_snapshot(&mut self, snap: Bm25Snapshot) {
        self.num_docs += snap.num_docs;
        self.doc_len.extend(snap.doc_len);
        self.total_doc_len = self.doc_len.values().map(|&l| u64::from(l)).sum();
        for (term, posts) in snap.postings {
            let df_add = snap.doc_freq.get(&term).copied().unwrap_or(0);
            let tid = *self.term_to_id.entry(term.clone()).or_insert_with(|| {
                let id = self.terms.len() as u32;
                self.terms.push(term);
                self.postings.push(Vec::new());
                self.doc_freq.push(0);
                id
            }) as usize;
            self.doc_freq[tid] += df_add;
            self.postings[tid].extend(posts);
        }
    }

    pub fn into_snapshot(self) -> Bm25Snapshot {
        let avg_dl = if self.num_docs > 0 {
            self.total_doc_len as f32 / self.num_docs as f32
        } else {
            0.0
        };
        let mut postings = HashMap::new();
        let mut doc_freq = HashMap::new();
        for (i, term) in self.terms.iter().enumerate() {
            postings.insert(term.clone(), self.postings[i].clone());
            doc_freq.insert(term.clone(), self.doc_freq[i]);
        }
        Bm25Snapshot {
            postings,
            doc_len: self.doc_len,
            doc_freq,
            num_docs: self.num_docs,
            avg_dl,
        }
    }
}

fn bm25_search(
    postings: &HashMap<String, Vec<(DocId, u32)>>,
    doc_len: &HashMap<DocId, u32>,
    doc_freq: &HashMap<String, u32>,
    num_docs: u32,
    avg_dl: f32,
    query: &str,
    k: usize,
) -> CandidateSet {
    let mut scores: HashMap<DocId, f32> = HashMap::new();
    let n = num_docs.max(1) as f32;
    for term in tokenize(query) {
        let Some(posts) = postings.get(&term) else {
            continue;
        };
        let df = *doc_freq.get(&term).unwrap_or(&0) as f32;
        let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
        for &(doc_id, tf) in posts {
            let dl = *doc_len.get(&doc_id).unwrap_or(&0) as f32;
            let tf = tf as f32;
            let denom = tf + K1 * (1.0 - B + B * dl / avg_dl.max(1.0));
            let score = idf * (tf * (K1 + 1.0)) / denom;
            *scores.entry(doc_id).or_default() += score;
        }
    }
    let mut ranked: Vec<(DocId, f32)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    ranked.truncate(k);
    let mut out = CandidateSet::with_capacity(ranked.len());
    for (id, score) in ranked {
        out.push(id, score);
    }
    out
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
        let bytes = crate::sparse::bm25_tbm3::encode_tbm3(&snap);
        let back = crate::sparse::bm25_tbm3::snapshot_from_tbm3(&bytes).unwrap();
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
