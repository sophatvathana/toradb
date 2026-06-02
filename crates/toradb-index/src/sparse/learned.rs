use std::collections::{BinaryHeap, HashMap};

use serde::{Deserialize, Serialize};
use toradb_core::{CandidateSet, DocId};

pub const MAX_QUERY_TOKENS: usize = 256;
pub const SEISMIC_POSTINGS_CAP: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparseProfile {
    Splade,
    Seismic,
}

impl SparseProfile {
    pub fn from_backend(s: &str) -> Option<Self> {
        match s {
            "splade" => Some(Self::Splade),
            "seismic" => Some(Self::Seismic),
            _ => None,
        }
    }

    fn postings_cap(self) -> Option<usize> {
        match self {
            Self::Splade => None,
            Self::Seismic => Some(SEISMIC_POSTINGS_CAP),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SparseInterner {
    terms: Vec<String>,
    #[serde(skip)]
    term_to_id: HashMap<String, u32>,
}

impl SparseInterner {
    pub fn intern(&mut self, term: &str) -> u32 {
        if let Some(&id) = self.term_to_id.get(term) {
            return id;
        }
        let id = self.terms.len() as u32;
        self.terms.push(term.to_string());
        self.term_to_id.insert(term.to_string(), id);
        id
    }

    pub fn lookup(&self, term: &str) -> Option<u32> {
        self.term_to_id.get(term).copied()
    }

    pub fn rebuild_index(&mut self) {
        self.term_to_id.clear();
        self.term_to_id.reserve(self.terms.len());
        for (id, term) in self.terms.iter().enumerate() {
            self.term_to_id.insert(term.clone(), id as u32);
        }
    }

    pub fn len(&self) -> usize {
        self.terms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn terms(&self) -> &[String] {
        &self.terms
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SparseSnapshot {
    pub interner: SparseInterner,
    pub postings: HashMap<u32, Vec<(DocId, f32)>>,
    pub num_docs: u32,
}

#[derive(Debug, Default)]
pub struct SparseWeightedIndex {
    interner: SparseInterner,
    postings: HashMap<u32, Vec<(DocId, f32)>>,
    num_docs: u32,
}

impl SparseWeightedIndex {
    pub fn doc_count(&self) -> u32 {
        self.num_docs
    }

    pub fn is_empty(&self) -> bool {
        self.num_docs == 0
    }

    pub fn add_document(&mut self, id: DocId, weights: &HashMap<String, f32>) {
        self.num_docs += 1;
        for (term, &w) in weights {
            if w == 0.0 {
                continue;
            }
            let tid = self.interner.intern(term);
            self.postings.entry(tid).or_default().push((id, w));
        }
    }

    pub fn encode_query(&self, weights: &HashMap<String, f32>) -> Vec<(u32, f32)> {
        let mut out = Vec::with_capacity(weights.len());
        for (term, &w) in weights {
            if w == 0.0 {
                continue;
            }
            if let Some(id) = self.interner.lookup(term) {
                out.push((id, w));
            }
        }
        out
    }

    pub fn search(&self, query: &[(u32, f32)], k: usize, profile: SparseProfile) -> CandidateSet {
        weighted_wand_topk(&self.postings, query, k, profile.postings_cap())
    }

    pub fn search_text(
        &self,
        weights: &HashMap<String, f32>,
        k: usize,
        profile: SparseProfile,
    ) -> CandidateSet {
        let q = self.encode_query(weights);
        self.search(&q, k, profile)
    }

    pub fn snapshot(&self) -> SparseSnapshot {
        SparseSnapshot {
            interner: self.interner.clone(),
            postings: self.postings.clone(),
            num_docs: self.num_docs,
        }
    }

    pub fn from_snapshot(mut snap: SparseSnapshot) -> Self {
        snap.interner.rebuild_index();
        let mut postings = snap.postings;
        for posts in postings.values_mut() {
            if posts.windows(2).any(|w| w[0].0 > w[1].0) {
                posts.sort_unstable_by_key(|&(id, _)| id);
            }
        }
        Self {
            interner: snap.interner,
            postings,
            num_docs: snap.num_docs,
        }
    }
}

struct TermCursor<'a> {
    posts: &'a [(DocId, f32)],
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

fn top_n_by_weight(posts: &[(DocId, f32)], cap: usize) -> Vec<(DocId, f32)> {
    let mut v = posts.to_vec();
    if v.len() > cap {
        v.sort_unstable_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        v.truncate(cap);
    }
    v.sort_unstable_by_key(|&(id, _)| id);
    v
}

fn weighted_wand_topk(
    postings: &HashMap<u32, Vec<(DocId, f32)>>,
    query: &[(u32, f32)],
    k: usize,
    postings_cap: Option<usize>,
) -> CandidateSet {
    if k == 0 || query.is_empty() {
        return CandidateSet::default();
    }

    struct Term<'a> {
        posts: &'a [(DocId, f32)],
        weight: f32,
        max_w: f32,
    }
    let mut terms: Vec<Term> = Vec::with_capacity(query.len());
    for &(tid, qw) in query {
        if qw == 0.0 {
            continue;
        }
        let Some(posts) = postings.get(&tid) else {
            continue;
        };
        if posts.is_empty() {
            continue;
        }
        let max_w = posts.iter().fold(0.0f32, |m, &(_, w)| m.max(w));
        terms.push(Term {
            posts,
            weight: qw,
            max_w,
        });
    }
    if terms.is_empty() {
        return CandidateSet::default();
    }
    if terms.len() > MAX_QUERY_TOKENS {
        terms.sort_by(|a, b| {
            b.weight
                .partial_cmp(&a.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        terms.truncate(MAX_QUERY_TOKENS);
    }

    let mut owned: Vec<Vec<(DocId, f32)>> = Vec::new();
    let mut plan: Vec<(usize, f32)> = Vec::with_capacity(terms.len()); // (owned_idx | MAX, weight)
    let mut borrowed: Vec<&[(DocId, f32)]> = Vec::new();
    for t in &terms {
        let needs_owned = postings_cap.is_some_and(|c| t.posts.len() > c);
        if needs_owned {
            let truncated = top_n_by_weight(t.posts, postings_cap.unwrap());
            plan.push((owned.len(), t.weight));
            owned.push(truncated);
        } else {
            plan.push((usize::MAX, t.weight));
            borrowed.push(t.posts);
        }
    }

    let mut cursors: Vec<TermCursor> = Vec::with_capacity(terms.len());
    let mut bi = 0usize;
    for (i, (owned_idx, weight)) in plan.iter().enumerate() {
        let posts: &[(DocId, f32)] = if *owned_idx == usize::MAX {
            let p = borrowed[bi];
            bi += 1;
            p
        } else {
            owned[*owned_idx].as_slice()
        };
        let max_w = if *owned_idx == usize::MAX {
            terms[i].max_w
        } else {
            posts.iter().fold(0.0f32, |m, &(_, w)| m.max(w))
        };
        cursors.push(TermCursor {
            posts,
            pos: 0,
            weight: *weight,
            max_contrib: *weight * max_w,
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
            let mut score = 0.0f32;
            for c in cursors.iter_mut() {
                if c.current() == Some(pivot_doc) {
                    score += c.weight * c.posts[c.pos].1;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, f32)]) -> HashMap<String, f32> {
        pairs.iter().map(|&(t, w)| (t.to_string(), w)).collect()
    }

    /// Brute-force weighted dot product over all docs, top-k by (score desc, doc asc).
    fn brute_force(
        docs: &[(DocId, HashMap<String, f32>)],
        query: &HashMap<String, f32>,
        k: usize,
    ) -> Vec<(DocId, f32)> {
        let mut scored: Vec<(DocId, f32)> = docs
            .iter()
            .filter_map(|(id, dw)| {
                let mut s = 0.0f32;
                for (t, &qw) in query {
                    if let Some(&w) = dw.get(t) {
                        s += qw * w;
                    }
                }
                if s > 0.0 {
                    Some((*id, s))
                } else {
                    None
                }
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        scored.truncate(k);
        scored
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn unit(&mut self) -> f32 {
            (self.next() % 1000) as f32 / 1000.0 + 0.001
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    #[test]
    fn weighted_wand_matches_brute_force_random() {
        let vocab: Vec<String> = (0..40).map(|i| format!("t{i}")).collect();
        let mut rng = Rng(0xdead_beef_1234_5678);
        let mut docs: Vec<(DocId, HashMap<String, f32>)> = Vec::new();
        let mut idx = SparseWeightedIndex::default();
        for id in 0..300u64 {
            let n_terms = 3 + rng.below(8);
            let mut m = HashMap::new();
            for _ in 0..n_terms {
                let term = vocab[rng.below(vocab.len())].clone();
                m.insert(term, rng.unit());
            }
            idx.add_document(id, &m);
            docs.push((id, m));
        }

        for _ in 0..40 {
            let qn = 1 + rng.below(6);
            let mut q = HashMap::new();
            for _ in 0..qn {
                q.insert(vocab[rng.below(vocab.len())].clone(), rng.unit());
            }
            let k = 10;
            let want = brute_force(&docs, &q, k);
            let got = idx.search_text(&q, k, SparseProfile::Splade);
            let got_pairs: Vec<(DocId, f32)> = got
                .ids
                .iter()
                .copied()
                .zip(got.scores.iter().copied())
                .collect();
            assert_eq!(got_pairs.len(), want.len(), "result count mismatch");
            for (i, ((gid, gs), (wid, ws))) in got_pairs.iter().zip(want.iter()).enumerate() {
                assert_eq!(*gid, *wid, "doc mismatch at rank {i}");
                assert!(
                    (gs - ws).abs() < 1e-4,
                    "score mismatch at rank {i}: {gs} vs {ws}"
                );
            }
        }
    }

    #[test]
    fn seismic_truncation_never_panics_and_returns_subset_topk() {
        let mut idx = SparseWeightedIndex::default();
        // One very long posting list to force truncation.
        for id in 0..2000u64 {
            let w = (id % 100) as f32 / 100.0 + 0.01;
            idx.add_document(id, &map(&[("hot", w)]));
        }
        let q = map(&[("hot", 1.0)]);
        let got = idx.search_text(&q, 10, SparseProfile::Seismic);
        assert_eq!(got.ids.len(), 10);
        // Highest-weight docs (id % 100 == 99) should dominate the top-k.
        for &id in &got.ids {
            assert_eq!(
                id % 100,
                99,
                "Seismic top-k should be the highest-weight postings"
            );
        }
    }

    #[test]
    fn interner_roundtrip() {
        let mut idx = SparseWeightedIndex::default();
        idx.add_document(0, &map(&[("alpha", 1.0), ("beta", 2.0)]));
        idx.add_document(1, &map(&[("beta", 0.5), ("gamma", 3.0)]));
        let snap = idx.snapshot();
        let restored = SparseWeightedIndex::from_snapshot(snap);
        let q = map(&[("beta", 1.0)]);
        let a = idx.search_text(&q, 5, SparseProfile::Splade);
        let b = restored.search_text(&q, 5, SparseProfile::Splade);
        assert_eq!(a.ids, b.ids);
    }

    #[test]
    fn query_drops_unseen_tokens() {
        let mut idx = SparseWeightedIndex::default();
        idx.add_document(0, &map(&[("alpha", 1.0)]));
        // "zeta" is not in the index — must be dropped, not panic.
        let q = map(&[("zeta", 5.0), ("alpha", 2.0)]);
        let got = idx.search_text(&q, 5, SparseProfile::Splade);
        assert_eq!(got.ids, vec![0]);
        assert!((got.scores[0] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn empty_query_returns_empty() {
        let mut idx = SparseWeightedIndex::default();
        idx.add_document(0, &map(&[("alpha", 1.0)]));
        let got = idx.search_text(&HashMap::new(), 5, SparseProfile::Splade);
        assert!(got.ids.is_empty());
    }
}
