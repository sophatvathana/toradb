//! BM25 on-disk format `TBM3`: sorted term dictionary + block-max posting lists (mmap-friendly).

use std::collections::HashMap;
use std::path::Path;

use memmap2::Mmap;
use toradb_core::{CandidateSet, DocId};

use super::bm25::{tokenize, B, Bm25Snapshot, K1};

pub fn decode_snapshot(bytes: &[u8]) -> Result<Bm25Snapshot, String> {
    snapshot_from_tbm3(bytes)
}

pub fn write_snapshot_file(path: &Path, snap: &Bm25Snapshot) -> Result<(), String> {
    write_tbm3_file(path, snap)
}

pub const TBM3_MAGIC: &[u8; 4] = b"TBM3";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 20;
const BLOCK_SIZE: usize = 128;

fn tf_component(tf: f32, avg_dl: f32) -> f32 {
    let dl = tf;
    let denom = tf + K1 * (1.0 - B + B * dl / avg_dl.max(1.0));
    (tf * (K1 + 1.0)) / denom
}

fn postings_sorted_by_doc_id(posts: &[(DocId, u32)]) -> bool {
    posts.windows(2).all(|w| w[0].0 <= w[1].0)
}

/// Append one term's posting list + block metadata to `blob`
fn append_posting_blob(
    blob: &mut Vec<u8>,
    posts: &[(DocId, u32)],
    avg_dl: f32,
    scratch: &mut Vec<(DocId, u32)>,
) {
    let sorted: &[(DocId, u32)] = if postings_sorted_by_doc_id(posts) {
        posts
    } else {
        scratch.clear();
        scratch.extend_from_slice(posts);
        scratch.sort_unstable_by_key(|(id, _)| *id);
        scratch.as_slice()
    };
    let n = sorted.len();
    let num_blocks = if n == 0 { 0 } else { n.div_ceil(BLOCK_SIZE) };
    blob.reserve(4 + n * 12 + 4 + num_blocks * 8);
    blob.extend_from_slice(&(n as u32).to_le_bytes());
    for &(doc_id, tf) in sorted {
        blob.extend_from_slice(&doc_id.to_le_bytes());
        blob.extend_from_slice(&tf.to_le_bytes());
    }
    blob.extend_from_slice(&(num_blocks as u32).to_le_bytes());
    for b in 0..num_blocks {
        let start = b * BLOCK_SIZE;
        let end = (start + BLOCK_SIZE).min(n);
        let mut block_max = 0.0f32;
        for &(_, tf) in &sorted[start..end] {
            block_max = block_max.max(tf_component(tf as f32, avg_dl));
        }
        blob.extend_from_slice(&(end as u32).to_le_bytes());
        blob.extend_from_slice(&block_max.to_le_bytes());
    }
}

pub fn encode_tbm3(snap: &Bm25Snapshot) -> Vec<u8> {
    let mut terms: Vec<&String> = snap.postings.keys().collect();
    terms.sort();
    let num_terms = terms.len() as u32;

    let mut dict = Vec::new();
    let mut postings_blob = Vec::new();
    let mut sort_scratch = Vec::new();
    for term in &terms {
        let posts = &snap.postings[*term];
        let offset = postings_blob.len() as u32;
        append_posting_blob(&mut postings_blob, posts, snap.avg_dl, &mut sort_scratch);
        let plen = postings_blob.len() as u32 - offset;
        let df = *snap.doc_freq.get(term.as_str()).unwrap_or(&(posts.len() as u32));
        let tb = term.as_bytes();
        let tlen = tb.len().min(u16::MAX as usize) as u16;
        dict.extend_from_slice(&tlen.to_le_bytes());
        dict.extend_from_slice(&tb[..tlen as usize]);
        dict.extend_from_slice(&df.to_le_bytes());
        dict.extend_from_slice(&offset.to_le_bytes());
        dict.extend_from_slice(&plen.to_le_bytes());
    }

    let mut out = Vec::with_capacity(HEADER_LEN + dict.len() + postings_blob.len());
    out.extend_from_slice(TBM3_MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&[0u8; 3]);
    out.extend_from_slice(&snap.num_docs.to_le_bytes());
    out.extend_from_slice(&snap.avg_dl.to_le_bytes());
    out.extend_from_slice(&num_terms.to_le_bytes());
    out.extend_from_slice(&dict);
    out.extend_from_slice(&postings_blob);
    out
}

pub fn write_tbm3_file(path: &Path, snap: &Bm25Snapshot) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = encode_tbm3(snap);
    let tmp = path.with_extension("tbm3.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

struct TermPostings {
    docs: Vec<(DocId, u32)>,
    block_ends: Vec<u32>,
    block_max_tf: Vec<f32>,
    idf: f32,
}

impl TermPostings {
    fn parse(bytes: &[u8], base: usize, num_docs: u32, df: u32) -> Result<Self, String> {
        if base + 4 > bytes.len() {
            return Err("truncated TBM3 postings".into());
        }
        let count = u32::from_le_bytes(bytes[base..base + 4].try_into().unwrap()) as usize;
        let mut pos = base + 4;
        let mut docs = Vec::with_capacity(count);
        for _ in 0..count {
            if pos + 12 > bytes.len() {
                return Err("truncated TBM3 doc postings".into());
            }
            let doc_id = u64::from_le_bytes(bytes[pos..pos + 8].try_into().unwrap());
            let tf = u32::from_le_bytes(bytes[pos + 8..pos + 12].try_into().unwrap());
            pos += 12;
            docs.push((doc_id, tf));
        }
        if pos + 4 > bytes.len() {
            return Err("truncated TBM3 blocks".into());
        }
        let num_blocks = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        let mut block_ends = Vec::with_capacity(num_blocks);
        let mut block_max_tf = Vec::with_capacity(num_blocks);
        for _ in 0..num_blocks {
            if pos + 8 > bytes.len() {
                return Err("truncated TBM3 block metadata".into());
            }
            block_ends.push(u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()));
            pos += 4;
            block_max_tf.push(f32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()));
            pos += 4;
        }
        let n = num_docs.max(1) as f32;
        let idf = ((n - df as f32 + 0.5) / (df as f32 + 0.5) + 1.0).ln();
        Ok(Self {
            docs,
            block_ends,
            block_max_tf,
            idf,
        })
    }

}

pub struct Bm25Tbm3View<'a> {
    bytes: &'a [u8],
    num_docs: u32,
    avg_dl: f32,
    num_terms: u32,
    dict_start: usize,
    postings_start: usize,
}

impl<'a> Bm25Tbm3View<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, String> {
        if bytes.len() < HEADER_LEN || &bytes[..4] != TBM3_MAGIC {
            return Err("invalid TBM3 magic".into());
        }
        if bytes[4] != VERSION {
            return Err(format!("unsupported TBM3 version {}", bytes[4]));
        }
        let num_docs = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let avg_dl = f32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let num_terms = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let mut dict_pos = 20usize;
        for _ in 0..num_terms {
            if dict_pos + 2 > bytes.len() {
                return Err("truncated TBM3 dict".into());
            }
            let tlen = u16::from_le_bytes(bytes[dict_pos..dict_pos + 2].try_into().unwrap()) as usize;
            dict_pos += 2 + tlen + 4 + 4 + 4;
        }
        Ok(Self {
            bytes,
            num_docs,
            avg_dl,
            num_terms,
            dict_start: 20,
            postings_start: dict_pos,
        })
    }

    pub fn from_mmap(mmap: &'a Mmap) -> Result<Self, String> {
        Self::open(mmap.as_ref())
    }

    pub(crate) fn term_at(&self, index: u32) -> Result<(&str, u32, u32, u32), String> {
        let mut pos = self.dict_start;
        for _ in 0..index {
            let tlen = u16::from_le_bytes(self.bytes[pos..pos + 2].try_into().unwrap()) as usize;
            pos += 2 + tlen + 4 + 4 + 4;
        }
        let tlen = u16::from_le_bytes(self.bytes[pos..pos + 2].try_into().unwrap()) as usize;
        pos += 2;
        let t = std::str::from_utf8(&self.bytes[pos..pos + tlen]).map_err(|e| e.to_string())?;
        pos += tlen;
        let df = u32::from_le_bytes(self.bytes[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let off = u32::from_le_bytes(self.bytes[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let plen = u32::from_le_bytes(self.bytes[pos..pos + 4].try_into().unwrap());
        Ok((t, df, off, plen))
    }

    fn find_term_entry(&self, term: &str) -> Option<(u32, u32, u32)> {
        let mut lo = 0u32;
        let mut hi = self.num_terms;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let (t, df, off, plen) = self.term_at(mid).ok()?;
            match t.cmp(term) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some((df, off, plen)),
            }
        }
        None
    }

    fn load_term(&self, df: u32, rel_off: u32) -> Result<TermPostings, String> {
        let base = self.postings_start + rel_off as usize;
        TermPostings::parse(self.bytes, base, self.num_docs, df)
    }

    pub(crate) fn brute_search(&self, query: &str, k: usize) -> CandidateSet {
        let mut scores: HashMap<DocId, f32> = HashMap::new();
        for term in tokenize(query) {
            let Some((df, rel_off, _plen)) = self.find_term_entry(&term) else {
                continue;
            };
            let Ok(list) = self.load_term(df, rel_off) else {
                continue;
            };
            for &(doc_id, tf) in &list.docs {
                let score = list.idf * tf_component(tf as f32, self.avg_dl);
                *scores.entry(doc_id).or_default() += score;
            }
        }
        top_k(scores, k)
    }

    pub fn search(&self, query: &str, k: usize) -> CandidateSet {
        if k == 0 {
            return CandidateSet::default();
        }
        let mut lists: Vec<TermPostings> = Vec::new();
        for term in tokenize(query) {
            let Some((df, rel_off, _plen)) = self.find_term_entry(&term) else {
                continue;
            };
            if let Ok(list) = self.load_term(df, rel_off) {
                if !list.docs.is_empty() {
                    lists.push(list);
                }
            }
        }
        if lists.is_empty() {
            return CandidateSet::default();
        }
        if lists.iter().map(|l| l.docs.len()).sum::<usize>() < 256 {
            return self.brute_search(query, k);
        }

        // MaxScore / block-max WAND: process terms shortest-first, skip blocks that
        // cannot reach the current top-k threshold.
        lists.sort_by_key(|l| l.docs.len());
        let term_ceiling: Vec<f32> = lists
            .iter()
            .map(|l| {
                l.idf
                    * l.block_max_tf
                        .iter()
                        .copied()
                        .fold(0.0f32, f32::max)
            })
            .collect();

        let mut scores: HashMap<DocId, f32> = HashMap::new();
        let mut threshold = 0.0f32;

        for (ti, list) in lists.iter().enumerate() {
            let remaining_ceiling: f32 = term_ceiling[ti + 1..].iter().sum();
            let mut block_idx = 0usize;
            let mut doc_idx = 0usize;
            while block_idx < list.block_ends.len() {
                let block_end = list.block_ends[block_idx] as usize;
                let block_ub = list.idf * list.block_max_tf[block_idx];
                let partial_max = list.docs[doc_idx..block_end]
                    .iter()
                    .map(|(id, _)| scores.get(id).copied().unwrap_or(0.0))
                    .fold(0.0f32, f32::max);
                let max_possible = partial_max + block_ub + remaining_ceiling;
                if max_possible < threshold {
                    doc_idx = block_end;
                    block_idx += 1;
                    continue;
                }
                for &(doc_id, tf) in &list.docs[doc_idx..block_end] {
                    let s = list.idf * tf_component(tf as f32, self.avg_dl);
                    *scores.entry(doc_id).or_default() += s;
                }
                doc_idx = block_end;
                block_idx += 1;
            }
            if scores.len() > k {
                let mut vals: Vec<f32> = scores.values().copied().collect();
                vals.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
                threshold = vals.get(k.saturating_sub(1)).copied().unwrap_or(0.0);
            }
        }

        top_k(scores, k)
    }
}

fn top_k(scores: HashMap<DocId, f32>, k: usize) -> CandidateSet {
    let mut ranked: Vec<(DocId, f32)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(k);
    let mut out = CandidateSet::with_capacity(ranked.len());
    for (id, score) in ranked {
        out.push(id, score);
    }
    out
}

/// Rebuild an in-memory snapshot (e.g. merge table-level index from segments).
pub fn snapshot_from_tbm3(bytes: &[u8]) -> Result<Bm25Snapshot, String> {
    let view = Bm25Tbm3View::open(bytes)?;
    let mut postings: HashMap<String, Vec<(DocId, u32)>> = HashMap::new();
    let mut doc_freq: HashMap<String, u32> = HashMap::new();
    let mut doc_len: HashMap<DocId, u32> = HashMap::new();
    for i in 0..view.num_terms {
        let (term, df, off, _plen) = view.term_at(i)?;
        doc_freq.insert(term.to_string(), df);
        let list = view.load_term(df, off)?;
        for &(doc_id, tf) in &list.docs {
            let entry = doc_len.entry(doc_id).or_insert(0);
            *entry = entry.saturating_add(tf);
        }
        postings.insert(term.to_string(), list.docs);
    }
    Ok(Bm25Snapshot {
        postings,
        doc_len,
        doc_freq,
        num_docs: view.num_docs,
        avg_dl: view.avg_dl,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::sparse::bm25::Bm25Snapshot;

    #[test]
    fn tbm3_roundtrip_search() {
        let snap = Bm25Snapshot::from_documents([(1u64, "Nikola Tesla motor"), (2, "wireless power")]);
        let bytes = encode_tbm3(&snap);
        let view = Bm25Tbm3View::open(&bytes).unwrap();
        let hits = view.search("Nikola motor", 5);
        assert!(!hits.is_empty());
    }

    #[test]
    fn tbm3_wand_matches_brute_on_small_corpus() {
        let mut docs = Vec::new();
        for i in 0..300u64 {
            docs.push((i, format!("term{} mixed document text {}", i % 17, i)));
        }
        let snap = Bm25Snapshot::from_documents(docs);
        let bytes = encode_tbm3(&snap);
        let view = Bm25Tbm3View::open(&bytes).unwrap();
        let query = "term5 mixed document";
        let wand = view.search(query, 10);
        let brute = view.brute_search(query, 10);
        let mut wand_pairs: Vec<_> = wand
            .ids
            .into_iter()
            .zip(wand.scores)
            .collect();
        let mut brute_pairs: Vec<_> = brute
            .ids
            .into_iter()
            .zip(brute.scores)
            .collect();
        wand_pairs.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        brute_pairs.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        assert_eq!(wand_pairs, brute_pairs);
    }

    #[test]
    fn tbm3_encode_unsorted_postings_roundtrip() {
        let mut postings = HashMap::new();
        postings.insert(
            "test".to_string(),
            vec![(3, 2), (1, 1), (2, 1)],
        );
        let snap = Bm25Snapshot {
            postings,
            doc_len: HashMap::new(),
            doc_freq: HashMap::from([("test".to_string(), 3)]),
            num_docs: 3,
            avg_dl: 1.0,
        };
        let bytes = encode_tbm3(&snap);
        let view = Bm25Tbm3View::open(&bytes).unwrap();
        let hits = view.search("test", 3);
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn tbm3_block_max_is_sound_upper_bound() {
        let snap = Bm25Snapshot::from_documents([(1u64, "alpha beta gamma"), (2, "beta only")]);
        let bytes = encode_tbm3(&snap);
        let view = Bm25Tbm3View::open(&bytes).unwrap();
        let (df, off, _) = view.find_term_entry("beta").unwrap();
        let list = view.load_term(df, off).unwrap();
        let idf = list.idf;
        for (bi, &end) in list.block_ends.iter().enumerate() {
            let start = if bi == 0 { 0 } else { list.block_ends[bi - 1] as usize };
            let end = end as usize;
            let mut true_max = 0.0f32;
            for &(doc_id, tf) in &list.docs[start..end] {
                let s = idf * tf_component(tf as f32, view.avg_dl);
                true_max = true_max.max(s);
                let _ = doc_id;
            }
            let stored = idf * list.block_max_tf[bi];
            assert!(
                stored + 1e-5 >= true_max,
                "block max {stored} < true max {true_max}"
            );
        }
    }
}
