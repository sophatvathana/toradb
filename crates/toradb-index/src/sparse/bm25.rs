use std::collections::{BinaryHeap, HashMap};

use serde::{Deserialize, Serialize};
use toradb_core::{CandidateSet, DocId};

pub(crate) const K1: f32 = 1.2;
pub(crate) const B: f32 = 0.75;

pub(crate) const MAX_QUERY_TERMS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bm25Params {
    pub k1: f32,
    pub b: f32,
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self { k1: K1, b: B }
    }
}

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
        self.search_with_params(query, k, Bm25Params::default())
    }

    pub fn search_with_params(&self, query: &str, k: usize, params: Bm25Params) -> CandidateSet {
        bm25_search(
            &self.postings,
            &self.doc_len,
            &self.doc_freq,
            self.num_docs,
            self.avg_dl,
            query,
            k,
            params,
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
        self.search_with_params(query, k, Bm25Params::default())
    }

    pub fn search_with_params(&self, query: &str, k: usize, params: Bm25Params) -> CandidateSet {
        bm25_search(
            &self.postings,
            &self.doc_len,
            &self.doc_freq,
            self.num_docs,
            self.avg_dl,
            query,
            k,
            params,
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

#[inline(always)]
fn term_doc_score(weight: f32, tf: u32, dl: f32, avg_dl: f32, params: Bm25Params) -> f32 {
    let tf = tf as f32;
    let denom = tf + params.k1 * (1.0 - params.b + params.b * dl / avg_dl.max(1.0));
    weight * (tf * (params.k1 + 1.0)) / denom
}

struct TermCursor<'a> {
    posts: &'a [(DocId, u32)],
    pos: usize,
    weight: f32,
    max_contrib: f32,
}

impl<'a> TermCursor<'a> {
    #[inline(always)]
    fn current(&self) -> Option<DocId> {
        self.posts.get(self.pos).map(|&(id, _)| id)
    }

    #[inline]
    fn advance_to(&mut self, target: DocId) {
        if self.pos >= self.posts.len() || self.posts[self.pos].0 >= target {
            return;
        }
        let mut step = 1usize;
        let mut lo = self.pos;
        while lo + step < self.posts.len() && self.posts[lo + step].0 < target {
            lo += step;
            step *= 2;
        }
        let hi = (lo + step + 1).min(self.posts.len());
        let mut left = lo;
        let mut right = hi;
        while left < right {
            let mid = left + (right - left) / 2;
            if self.posts[mid].0 < target {
                left = mid + 1;
            } else {
                right = mid;
            }
        }
        self.pos = left;
    }
}

#[derive(PartialEq)]
struct HeapItem {
    score: f32,
    doc: DocId,
}
impl Eq for HeapItem {}
impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .score
            .partial_cmp(&self.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(self.doc.cmp(&other.doc))
    }
}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[allow(clippy::too_many_arguments)]
fn bm25_search(
    postings: &HashMap<String, Vec<(DocId, u32)>>,
    doc_len: &HashMap<DocId, u32>,
    doc_freq: &HashMap<String, u32>,
    num_docs: u32,
    avg_dl: f32,
    query: &str,
    k: usize,
    params: Bm25Params,
) -> CandidateSet {
    if k == 0 {
        return CandidateSet::default();
    }
    let n = num_docs.max(1) as f32;

    let mut qtf: HashMap<&str, u32> = HashMap::new();
    let toks = tokenize(query);
    for term in &toks {
        *qtf.entry(term.as_str()).or_default() += 1;
    }

    let mut terms: Vec<(&str, f32, f32)> = qtf
        .into_iter()
        .filter_map(|(term, qf)| {
            let posts = postings.get(term)?;
            if posts.is_empty() {
                return None;
            }
            let df = *doc_freq.get(term).unwrap_or(&0) as f32;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            Some((term, idf, idf * qf as f32))
        })
        .collect();
    if terms.is_empty() {
        return CandidateSet::default();
    }
    if terms.len() > MAX_QUERY_TERMS {
        terms.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(b.0))
        });
        terms.truncate(MAX_QUERY_TERMS);
    }

    let mut owned: Vec<Vec<(DocId, u32)>> = Vec::new();
    let mut sources: Vec<(usize, f32)> = Vec::with_capacity(terms.len()); // (owned_idx or usize::MAX, weight)
    let mut borrowed: Vec<(&[(DocId, u32)], f32)> = Vec::with_capacity(terms.len());
    for (term, _idf, weight) in &terms {
        let raw = &postings[*term];
        if raw.windows(2).all(|w| w[0].0 <= w[1].0) {
            sources.push((usize::MAX, *weight));
            borrowed.push((raw.as_slice(), *weight));
        } else {
            let mut s = raw.clone();
            s.sort_unstable_by_key(|&(id, _)| id);
            sources.push((owned.len(), *weight));
            owned.push(s);
        }
    }
    let mut cursors: Vec<TermCursor> = Vec::with_capacity(terms.len());
    let mut bi = 0usize;
    for (owned_idx, weight) in &sources {
        let posts: &[(DocId, u32)] = if *owned_idx == usize::MAX {
            let p = borrowed[bi].0;
            bi += 1;
            p
        } else {
            owned[*owned_idx].as_slice()
        };
        cursors.push(TermCursor {
            posts,
            pos: 0,
            weight: *weight,
            max_contrib: *weight * (params.k1 + 1.0),
        });
    }

    let mut heap: BinaryHeap<HeapItem> = BinaryHeap::with_capacity(k + 1);
    let mut threshold = f32::NEG_INFINITY;

    #[inline(always)]
    fn key(c: &TermCursor) -> u64 {
        c.current().map(|id| id as u64).unwrap_or(u64::MAX)
    }
    #[inline]
    fn resort(cursors: &mut [TermCursor]) {
        for i in 1..cursors.len() {
            let mut j = i;
            while j > 0 && key(&cursors[j - 1]) > key(&cursors[j]) {
                cursors.swap(j - 1, j);
                j -= 1;
            }
        }
    }
    resort(&mut cursors);

    loop {
        if cursors[0].current().is_none() {
            break;
        }

        let mut acc = 0.0f32;
        let mut pivot = None;
        for (i, c) in cursors.iter().enumerate() {
            if c.current().is_none() {
                break;
            }
            acc += c.max_contrib;
            if acc > threshold {
                pivot = Some(i);
                break;
            }
        }
        let Some(pivot) = pivot else {
            break;
        };
        let pivot_doc = cursors[pivot].current().unwrap();

        if cursors[0].current() == Some(pivot_doc) {
            let dl = *doc_len.get(&pivot_doc).unwrap_or(&0) as f32;
            let mut score = 0.0f32;
            for c in cursors.iter_mut() {
                if c.current() == Some(pivot_doc) {
                    let tf = c.posts[c.pos].1;
                    score += term_doc_score(c.weight, tf, dl, avg_dl, params);
                    c.pos += 1;
                } else {
                    break;
                }
            }
            if heap.len() < k {
                heap.push(HeapItem {
                    score,
                    doc: pivot_doc,
                });
                if heap.len() == k {
                    threshold = heap.peek().map(|h| h.score).unwrap_or(threshold);
                }
            } else if score > threshold
                || (score == threshold && heap.peek().is_some_and(|h| pivot_doc < h.doc))
            {
                heap.pop();
                heap.push(HeapItem {
                    score,
                    doc: pivot_doc,
                });
                threshold = heap.peek().map(|h| h.score).unwrap_or(threshold);
            }
        } else {
            for c in cursors.iter_mut() {
                match c.current() {
                    Some(d) if d < pivot_doc => c.advance_to(pivot_doc),
                    _ => {}
                }
            }
        }
        resort(&mut cursors);
    }

    let mut ranked: Vec<HeapItem> = heap.into_vec();
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.doc.cmp(&b.doc))
    });
    let mut out = CandidateSet::with_capacity(ranked.len());
    for item in ranked {
        out.push(item.doc, item.score);
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
    use super::{tokenize, Bm25Index, Bm25Params, Bm25Snapshot};

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

    fn ranking(set: &toradb_core::CandidateSet) -> Vec<u64> {
        set.ids.clone()
    }

    #[test]
    fn repeated_terms_match_distinct_ranking() {
        let snap = Bm25Snapshot::from_documents([
            (0u64, "alpha beta gamma"),
            (1u64, "alpha alpha beta"),
            (2u64, "gamma delta"),
        ]);
        let index = Bm25Index::from_snapshot(snap);

        let distinct = index.search("alpha beta", 10);
        let repeated = index.search("alpha beta alpha beta beta", 10);
        assert_eq!(ranking(&distinct), ranking(&repeated));
    }

    fn brute_force(index: &Bm25Index, query: &str, k: usize) -> Vec<(u64, f32)> {
        use super::{tokenize, B, K1, MAX_QUERY_TERMS};
        let snap = index.snapshot();
        let n = snap.num_docs.max(1) as f32;
        let mut qtf: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for t in tokenize(query) {
            *qtf.entry(t).or_default() += 1;
        }
        let mut terms: Vec<(String, f32, f32)> = qtf
            .into_iter()
            .filter(|(t, _)| snap.postings.contains_key(t))
            .map(|(t, qf)| {
                let df = *snap.doc_freq.get(&t).unwrap_or(&0) as f32;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                (t, idf, idf * qf as f32)
            })
            .collect();
        if terms.len() > MAX_QUERY_TERMS {
            terms.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
            terms.truncate(MAX_QUERY_TERMS);
        }
        let mut scores: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();
        for (t, _idf, weight) in &terms {
            for &(doc, tf) in &snap.postings[t] {
                let dl = *snap.doc_len.get(&doc).unwrap_or(&0) as f32;
                let tf = tf as f32;
                let denom = tf + K1 * (1.0 - B + B * dl / snap.avg_dl.max(1.0));
                *scores.entry(doc).or_default() += weight * (tf * (K1 + 1.0)) / denom;
            }
        }
        let mut v: Vec<(u64, f32)> = scores.into_iter().collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
        v.truncate(k);
        v
    }

    #[test]
    fn wand_matches_brute_force_random() {
        let mut state = 0x9e3779b97f4a7c15u64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let vocab: Vec<String> = (0..200).map(|i| format!("t{i}")).collect();
        let mut docs: Vec<(u64, String)> = Vec::new();
        for d in 0..2000u64 {
            let len = 5 + (rng() % 40) as usize;
            let text: String = (0..len)
                .map(|_| {
                    let r = (rng() % vocab.len() as u64) as usize;
                    let idx = (r * r) / vocab.len();
                    vocab[idx.min(vocab.len() - 1)].clone()
                })
                .collect::<Vec<_>>()
                .join(" ");
            docs.push((d, text));
        }
        let snap = Bm25Snapshot::from_documents(docs.iter().map(|(id, t)| (*id, t.as_str())));
        let index = Bm25Index::from_snapshot(snap);

        for _ in 0..200 {
            let qlen = 1 + (rng() % 50) as usize;
            let query: String = (0..qlen)
                .map(|_| {
                    let r = (rng() % vocab.len() as u64) as usize;
                    let idx = (r * r) / vocab.len();
                    vocab[idx.min(vocab.len() - 1)].clone()
                })
                .collect::<Vec<_>>()
                .join(" ");
            let k = 1 + (rng() % 30) as usize;
            let got = index.search(&query, k);
            let want = brute_force(&index, &query, k);
            assert_eq!(
                got.ids,
                want.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                "ranking mismatch for query={query:?} k={k}"
            );
            for (i, (_, ws)) in want.iter().enumerate() {
                assert!(
                    (got.scores[i] - ws).abs() < 1e-4,
                    "score mismatch at {i} for query={query:?}: got {} want {}",
                    got.scores[i],
                    ws
                );
            }
        }
    }

    #[test]
    fn wand_handles_unsorted_postings() {
        let a = Bm25Snapshot::from_documents([(5u64, "alpha beta"), (9u64, "alpha")]);
        let b = Bm25Snapshot::from_documents([(2u64, "alpha beta beta"), (7u64, "beta")]);
        let mut merged = a;
        merged.merge(b);
        let index = Bm25Index::from_snapshot(merged);
        let got = index.search("alpha beta", 10);
        let want = brute_force(&index, "alpha beta", 10);
        assert_eq!(got.ids, want.iter().map(|(id, _)| *id).collect::<Vec<_>>());
    }

    #[test]
    fn long_query_scores_only_highest_idf_terms() {
        use super::MAX_QUERY_TERMS;
        let mut docs: Vec<(u64, String)> = Vec::new();
        let common: Vec<String> = (0..MAX_QUERY_TERMS + 20)
            .map(|i| format!("common{i}"))
            .collect();
        let filler = common.join(" ");
        for id in 0..50u64 {
            docs.push((id, filler.clone()));
        }
        docs.push((999u64, "rareword filler text".to_string()));
        let snap = Bm25Snapshot::from_documents(docs.iter().map(|(id, t)| (*id, t.as_str())));
        let index = Bm25Index::from_snapshot(snap);
        let query = format!("rareword {filler}");
        let res = index.search(&query, 5);
        assert_eq!(res.ids.first(), Some(&999u64));
    }

    #[test]
    fn default_params_match_constants() {
        let docs = [
            (0u64, "tesla tesla tesla motor"),
            (1u64, "tesla motor design"),
        ];
        let snap = Bm25Snapshot::from_documents(docs.iter().map(|(id, t)| (*id, *t)));
        let idx = Bm25Index::from_snapshot(snap);
        let a = idx.search("tesla motor", 5);
        let b = idx.search_with_params("tesla motor", 5, Bm25Params::default());
        assert_eq!(a.ids, b.ids);
        for (x, y) in a.scores.iter().zip(b.scores.iter()) {
            assert!((x - y).abs() < 1e-6);
        }
    }

    #[test]
    fn params_change_scores() {
        let docs = [
            (0u64, "tesla tesla tesla tesla tesla motor"),
            (1u64, "tesla motor"),
        ];
        let snap = Bm25Snapshot::from_documents(docs.iter().map(|(id, t)| (*id, *t)));
        let idx = Bm25Index::from_snapshot(snap);
        let low = idx.search_with_params("tesla", 5, Bm25Params { k1: 0.1, b: 0.75 });
        let high = idx.search_with_params("tesla", 5, Bm25Params { k1: 5.0, b: 0.75 });
        let s0_low = low.scores[low.ids.iter().position(|&i| i == 0).unwrap()];
        let s0_high = high.scores[high.ids.iter().position(|&i| i == 0).unwrap()];
        assert!(
            s0_high > s0_low,
            "higher k1 should raise the high-tf doc score: {s0_high} vs {s0_low}"
        );
    }
}
