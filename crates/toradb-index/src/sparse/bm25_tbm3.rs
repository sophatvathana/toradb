//! BM25 on-disk format `TBM3`: sorted term dictionary + compressed block-max
//! posting lists (PFOR-128 delta doc-ids + varbyte tfs), mmap-friendly and
//! zero-copy at query time.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::path::Path;

use memmap2::Mmap;
use rayon::prelude::*;
use toradb_core::{CandidateSet, DocId};

const PARALLEL_BLOCK_THRESHOLD: usize = 2048;
const MIN_BLOCKS_PER_WORKER: usize = 512;

use super::bm25::{tokenize, Bm25Snapshot, B, K1, MAX_QUERY_TERMS};

pub fn decode_snapshot(bytes: &[u8]) -> Result<Bm25Snapshot, String> {
    snapshot_from_tbm3(bytes)
}

pub fn write_snapshot_file(path: &Path, snap: &Bm25Snapshot) -> Result<(), String> {
    write_tbm3_file(path, snap)
}

pub const TBM3_MAGIC: &[u8; 4] = b"TBM3";
pub const TBM3_VERSION: u8 = 2;
const VERSION: u8 = TBM3_VERSION;
const HEADER_LEN: usize = 20;
const BLOCK_SIZE: usize = 128;

pub fn read_tbm3_version(path: &Path) -> Option<u8> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 5];
    f.read_exact(&mut buf).ok()?;
    if &buf[..4] != TBM3_MAGIC {
        return None;
    }
    Some(buf[4])
}

#[inline(always)]
fn tf_component(tf: f32, avg_dl: f32) -> f32 {
    let dl = tf;
    let denom = tf + K1 * (1.0 - B + B * dl / avg_dl.max(1.0));
    (tf * (K1 + 1.0)) / denom
}

fn postings_sorted_by_doc_id(posts: &[(DocId, u32)]) -> bool {
    posts.windows(2).all(|w| w[0].0 <= w[1].0)
}

fn varbyte_encode(mut v: u32, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push(((v as u8) & 0x7F) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

#[inline(always)]
fn varbyte_decode(bytes: &[u8], pos: &mut usize) -> u32 {
    let mut v: u32 = 0;
    let mut shift: u32 = 0;
    loop {
        let b = bytes[*pos];
        *pos += 1;
        v |= ((b & 0x7F) as u32) << shift;
        if b < 0x80 {
            return v;
        }
        shift += 7;
    }
}

fn bits_needed(max: u32) -> u8 {
    if max == 0 {
        return 0;
    }
    32 - max.leading_zeros() as u8
}

fn bitpack(values: &[u32], bits: u8, out: &mut Vec<u8>) {
    if bits == 0 {
        return;
    }
    let mut acc: u64 = 0;
    let mut filled: u32 = 0;
    let bits = bits as u32;
    let mask: u64 = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    for &v in values {
        acc |= ((v as u64) & mask) << filled;
        filled += bits;
        while filled >= 8 {
            out.push((acc & 0xFF) as u8);
            acc >>= 8;
            filled -= 8;
        }
    }
    if filled > 0 {
        out.push((acc & 0xFF) as u8);
    }
}

#[inline(always)]
fn bitunpack_into(payload: &[u8], bits: u8, count: usize, out: &mut [u32; BLOCK_SIZE]) {
    if bits == 0 {
        for slot in out.iter_mut().take(count) {
            *slot = 0;
        }
        return;
    }
    let bits = bits as u32;
    let mask: u64 = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let mut acc: u64 = 0;
    let mut filled: u32 = 0;
    let mut byte_idx: usize = 0;
    for slot in out.iter_mut().take(count) {
        while filled < bits {
            // Defensive: extend with zero if payload exhausted (shouldn't happen).
            let b = if byte_idx < payload.len() {
                payload[byte_idx]
            } else {
                0
            };
            acc |= (b as u64) << filled;
            byte_idx += 1;
            filled += 8;
        }
        *slot = (acc & mask) as u32;
        acc >>= bits;
        filled -= bits;
    }
}

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

    blob.extend_from_slice(&(n as u32).to_le_bytes());
    blob.extend_from_slice(&(num_blocks as u32).to_le_bytes());

    let mut prev_doc: DocId = 0;
    let mut deltas: Vec<u32> = Vec::with_capacity(BLOCK_SIZE);
    let mut tf_payload: Vec<u8> = Vec::with_capacity(BLOCK_SIZE);
    let mut docid_payload: Vec<u8> = Vec::with_capacity(BLOCK_SIZE * 4);

    for b in 0..num_blocks {
        let start = b * BLOCK_SIZE;
        let end = (start + BLOCK_SIZE).min(n);
        let count = end - start;

        deltas.clear();
        tf_payload.clear();
        docid_payload.clear();

        let mut max_delta: u32 = 0;
        let mut block_max_tf = 0.0f32;
        let mut local_prev = prev_doc;
        for &(doc_id, tf) in &sorted[start..end] {
            let d = (doc_id - local_prev) as u32;
            local_prev = doc_id;
            if d > max_delta {
                max_delta = d;
            }
            deltas.push(d);
            varbyte_encode(tf.saturating_sub(1), &mut tf_payload);
            block_max_tf = block_max_tf.max(tf_component(tf as f32, avg_dl));
        }
        let bits = bits_needed(max_delta);
        bitpack(&deltas, bits, &mut docid_payload);

        let block_last_docid = sorted[end - 1].0;
        blob.extend_from_slice(&block_last_docid.to_le_bytes());
        blob.extend_from_slice(&(count as u32).to_le_bytes());
        blob.extend_from_slice(&block_max_tf.to_le_bytes());
        blob.push(bits);
        blob.extend_from_slice(&(docid_payload.len() as u32).to_le_bytes());
        blob.extend_from_slice(&(tf_payload.len() as u32).to_le_bytes());
        blob.extend_from_slice(&docid_payload);
        blob.extend_from_slice(&tf_payload);

        prev_doc = block_last_docid;
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
        let df = *snap
            .doc_freq
            .get(term.as_str())
            .unwrap_or(&(posts.len() as u32));
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

#[derive(Clone, Copy)]
struct BlockHeader {
    block_last_docid: DocId,
    count: u32,
    block_max_tf: f32,
    docid_bits: u8,
    docid_payload_off: usize,
    tf_payload_off: usize,
    tf_payload_len: usize,
    next_block_off: usize,
}

#[inline(always)]
fn read_block_header(bytes: &[u8], pos: usize) -> Result<BlockHeader, String> {
    if pos + 8 + 4 + 4 + 1 + 4 + 4 > bytes.len() {
        return Err("truncated TBM3 block header".into());
    }
    let block_last_docid = u64::from_le_bytes(bytes[pos..pos + 8].try_into().unwrap());
    let mut p = pos + 8;
    let count = u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
    p += 4;
    let block_max_tf = f32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
    p += 4;
    let docid_bits = bytes[p];
    p += 1;
    let docid_len = u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap()) as usize;
    p += 4;
    let tf_len = u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap()) as usize;
    p += 4;
    let docid_off = p;
    let tf_off = docid_off + docid_len;
    let next_off = tf_off + tf_len;
    if next_off > bytes.len() {
        return Err("truncated TBM3 block payload".into());
    }
    Ok(BlockHeader {
        block_last_docid,
        count,
        block_max_tf,
        docid_bits,
        docid_payload_off: docid_off,
        tf_payload_off: tf_off,
        tf_payload_len: tf_len,
        next_block_off: next_off,
    })
}

pub struct TermPostings {
    pub docs: Vec<(DocId, u32)>,
    pub block_ends: Vec<u32>,
    pub block_max_tf: Vec<f32>,
    pub idf: f32,
}

impl TermPostings {
    fn parse(bytes: &[u8], base: usize, num_docs: u32, df: u32) -> Result<Self, String> {
        if base + 8 > bytes.len() {
            return Err("truncated TBM3 postings".into());
        }
        let total = u32::from_le_bytes(bytes[base..base + 4].try_into().unwrap()) as usize;
        let num_blocks = u32::from_le_bytes(bytes[base + 4..base + 8].try_into().unwrap()) as usize;
        let mut docs = Vec::with_capacity(total);
        let mut block_ends = Vec::with_capacity(num_blocks);
        let mut block_max_tf = Vec::with_capacity(num_blocks);
        let mut pos = base + 8;
        let mut prev: DocId = 0;
        let mut deltas: [u32; BLOCK_SIZE] = [0u32; BLOCK_SIZE];
        for _ in 0..num_blocks {
            let h = read_block_header(bytes, pos)?;
            let count = h.count as usize;
            bitunpack_into(
                &bytes[h.docid_payload_off
                    ..h.docid_payload_off
                        + (h.next_block_off - h.docid_payload_off - h.tf_payload_len)],
                h.docid_bits,
                count,
                &mut deltas,
            );
            let tf_slice = &bytes[h.tf_payload_off..h.tf_payload_off + h.tf_payload_len];
            let mut tf_pos = 0usize;
            for i in 0..count {
                prev = prev.wrapping_add(deltas[i] as u64);
                let tf = varbyte_decode(tf_slice, &mut tf_pos).saturating_add(1);
                docs.push((prev, tf));
            }
            block_ends.push(docs.len() as u32);
            block_max_tf.push(h.block_max_tf);
            pos = h.next_block_off;
        }
        let _ = total;
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

#[derive(Clone, Copy, Debug)]
struct DictEntry {
    term_off: usize,
    term_len: u16,
    df: u32,
    post_off: u32,
    post_len: u32,
}

#[derive(Debug, Clone)]
pub struct Bm25Tbm3Meta {
    num_docs: u32,
    avg_dl: f32,
    num_terms: u32,
    postings_start: usize,
    dict: Vec<DictEntry>,
}

impl Bm25Tbm3Meta {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < HEADER_LEN || &bytes[..4] != TBM3_MAGIC {
            return Err("invalid TBM3 magic".into());
        }
        if bytes[4] != VERSION {
            return Err(format!(
                "unsupported TBM3 version {} (expected {})",
                bytes[4], VERSION
            ));
        }
        let num_docs = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let avg_dl = f32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let num_terms = u32::from_le_bytes(bytes[16..20].try_into().unwrap());

        let mut dict = Vec::with_capacity(num_terms as usize);
        let mut pos = 20usize;
        for _ in 0..num_terms {
            if pos + 2 > bytes.len() {
                return Err("truncated TBM3 dict".into());
            }
            let tlen = u16::from_le_bytes(bytes[pos..pos + 2].try_into().unwrap());
            pos += 2;
            let term_off = pos;
            pos += tlen as usize;
            if pos + 12 > bytes.len() {
                return Err("truncated TBM3 dict entry".into());
            }
            let df = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
            pos += 4;
            let post_off = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
            pos += 4;
            let post_len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
            pos += 4;
            dict.push(DictEntry {
                term_off,
                term_len: tlen,
                df,
                post_off,
                post_len,
            });
        }
        Ok(Self {
            num_docs,
            avg_dl,
            num_terms,
            postings_start: pos,
            dict,
        })
    }
}

pub struct Bm25Tbm3View<'a> {
    bytes: &'a [u8],
    num_docs: u32,
    avg_dl: f32,
    num_terms: u32,
    postings_start: usize,
    dict_owned: Option<Vec<DictEntry>>,
    dict_ref: Option<&'a [DictEntry]>,
}

impl<'a> Bm25Tbm3View<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, String> {
        let meta = Bm25Tbm3Meta::parse(bytes)?;
        Ok(Self {
            bytes,
            num_docs: meta.num_docs,
            avg_dl: meta.avg_dl,
            num_terms: meta.num_terms,
            postings_start: meta.postings_start,
            dict_owned: Some(meta.dict),
            dict_ref: None,
        })
    }

    /// Build a view from already-parsed metadata. Avoids re-walking the dict
    /// for each query when the segment is cached across calls.
    pub fn with_meta(bytes: &'a [u8], meta: &'a Bm25Tbm3Meta) -> Self {
        Self {
            bytes,
            num_docs: meta.num_docs,
            avg_dl: meta.avg_dl,
            num_terms: meta.num_terms,
            postings_start: meta.postings_start,
            dict_owned: None,
            dict_ref: Some(&meta.dict),
        }
    }

    #[inline]
    fn dict(&self) -> &[DictEntry] {
        if let Some(d) = self.dict_ref {
            d
        } else {
            self.dict_owned.as_deref().unwrap_or(&[])
        }
    }

    pub fn from_mmap(mmap: &'a Mmap) -> Result<Self, String> {
        Self::open(mmap.as_ref())
    }

    #[inline]
    fn term_str(&self, e: &DictEntry) -> &str {
        // Safe: dict was constructed from the same bytes; UTF-8 was enforced on
        // write (terms come from `tokenize`, which produces valid UTF-8).
        unsafe {
            std::str::from_utf8_unchecked(&self.bytes[e.term_off..e.term_off + e.term_len as usize])
        }
    }

    pub(crate) fn term_at(&self, index: u32) -> Result<(&str, u32, u32, u32), String> {
        let e = self
            .dict()
            .get(index as usize)
            .ok_or_else(|| "term index out of range".to_string())?;
        Ok((self.term_str(e), e.df, e.post_off, e.post_len))
    }

    pub(crate) fn find_term_entry(&self, term: &str) -> Option<(u32, u32, u32)> {
        let dict = self.dict();
        let idx = dict.binary_search_by(|e| self.term_str(e).cmp(term)).ok()?;
        let e = dict[idx];
        Some((e.df, e.post_off, e.post_len))
    }

    pub(crate) fn load_term(&self, df: u32, rel_off: u32) -> Result<TermPostings, String> {
        let base = self.postings_start + rel_off as usize;
        TermPostings::parse(self.bytes, base, self.num_docs, df)
    }

    fn open_cursor(&self, df: u32, rel_off: u32) -> Result<TermCursor<'a>, String> {
        TermCursor::open(
            self.bytes,
            self.postings_start + rel_off as usize,
            df,
            self.num_docs,
            self.avg_dl,
        )
    }

    pub fn brute_search(&self, query: &str, k: usize) -> CandidateSet {
        // Single unified path: just call search; it falls back to per-term linear scan
        // if appropriate. Kept public for tests that compare against MaxScore.
        let mut cursors = self.cursors_for(query);
        if cursors.is_empty() {
            return CandidateSet::default();
        }
        let mut scores: HashMap<DocId, f32> = HashMap::new();
        let mut deltas = [0u32; BLOCK_SIZE];
        for c in cursors.iter_mut() {
            let idf = c.idf;
            while c.has_block() {
                c.decode_current_block(&mut deltas);
                let count = c.block_count();
                let decoded = c.decoded.as_ref().unwrap();
                for i in 0..count {
                    let doc_id = decoded.doc_ids[i];
                    let tf = decoded.tfs[i];
                    let s = idf * tf_component(tf as f32, self.avg_dl);
                    *scores.entry(doc_id).or_default() += s;
                }
                c.advance_block();
            }
        }
        scores_to_topk(scores, k)
    }

    fn cursors_for(&self, query: &str) -> Vec<TermCursor<'a>> {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut cursors = Vec::new();
        for term in tokenize(query) {
            if !seen.insert(term.clone()) {
                continue;
            }
            if let Some((df, rel_off, _plen)) = self.find_term_entry(&term) {
                if let Ok(c) = self.open_cursor(df, rel_off) {
                    if c.total_docs > 0 {
                        cursors.push(c);
                    }
                }
            }
        }
        if cursors.len() > MAX_QUERY_TERMS {
            cursors.sort_by(|a, b| {
                b.idf
                    .partial_cmp(&a.idf)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.total_docs.cmp(&b.total_docs))
            });
            cursors.truncate(MAX_QUERY_TERMS);
        }
        cursors
    }

    pub fn search(&self, query: &str, k: usize) -> CandidateSet {
        if k == 0 {
            return CandidateSet::default();
        }
        let mut cursors = self.cursors_for(query);
        if cursors.is_empty() {
            return CandidateSet::default();
        }
        if cursors.len() == 1 {
            // Single-term: stream straight into a bounded min-heap of size k.
            return single_term_topk(&mut cursors[0], k, self.avg_dl);
        }
        // Sort by document frequency ascending: cheaper terms first.
        cursors.sort_by_key(|c| c.total_docs);

        let plans: Vec<std::sync::Arc<TermPlan>> = cursors
            .iter()
            .map(|c| std::sync::Arc::new(TermPlan::build(c)))
            .collect();

        let max_blocks = plans.iter().map(|p| p.blocks.len()).max().unwrap_or(0);
        let workers = rayon::current_num_threads();
        if workers <= 1 || max_blocks <= PARALLEL_BLOCK_THRESHOLD {
            return block_max_wand(cursors, &plans, k, self.avg_dl, 0, DocId::MAX);
        }

        let max_doc = plans
            .iter()
            .filter_map(|p| p.blocks.last().map(|b| b.last_docid))
            .max()
            .unwrap_or(0);
        let parts = workers.min(8).max(1);
        let span = (max_doc / parts as u64).max(1);
        let ranges: Vec<(DocId, DocId)> = (0..parts)
            .map(|w| {
                let lo = w as u64 * span;
                let hi = if w + 1 == parts {
                    DocId::MAX
                } else {
                    (w as u64 + 1) * span
                };
                (lo, hi)
            })
            .filter(|(lo, hi)| lo < hi)
            .collect();

        let avg_dl = self.avg_dl;
        let cursors_ref = &cursors;
        let plans_ref = &plans;
        let partials: Vec<CandidateSet> = ranges
            .par_iter()
            .map(|&(lo, hi)| {
                let local: Vec<TermCursor> = cursors_ref.iter().map(|c| c.reopen()).collect();
                block_max_wand(local, plans_ref, k, avg_dl, lo, hi)
            })
            .collect();

        let mut merged: BinaryHeap<Reverse<ScoredDoc>> = BinaryHeap::with_capacity(k + 1);
        for set in &partials {
            for (i, &doc_id) in set.ids.iter().enumerate() {
                push_topk(
                    &mut merged,
                    ScoredDoc {
                        score: set.scores[i],
                        doc_id,
                    },
                    k,
                );
            }
        }
        heap_to_candidates(merged)
    }
}

fn score_range_into_heap(
    cursor: &mut TermCursor<'_>,
    k: usize,
    avg_dl: f32,
) -> BinaryHeap<Reverse<ScoredDoc>> {
    let mut heap: BinaryHeap<Reverse<ScoredDoc>> = BinaryHeap::with_capacity(k + 1);
    let mut deltas = [0u32; BLOCK_SIZE];
    let idf = cursor.idf;
    while cursor.has_block() {
        cursor.decode_current_block(&mut deltas);
        let count = cursor.block_count();
        let decoded = cursor.decoded.as_ref().unwrap();
        for i in 0..count {
            let doc_id = decoded.doc_ids[i];
            let tf = decoded.tfs[i];
            let s = idf * tf_component(tf as f32, avg_dl);
            push_topk(&mut heap, ScoredDoc { score: s, doc_id }, k);
        }
        cursor.advance_block();
    }
    heap
}

fn single_term_topk(cursor: &mut TermCursor<'_>, k: usize, avg_dl: f32) -> CandidateSet {
    let num_blocks = cursor.num_blocks;

    // Small posting lists: stay sequential, no thread overhead.
    if num_blocks <= PARALLEL_BLOCK_THRESHOLD {
        let heap = score_range_into_heap(cursor, k, avg_dl);
        return heap_to_candidates(heap);
    }

    let workers = rayon::current_num_threads()
        .min(num_blocks / MIN_BLOCKS_PER_WORKER)
        .max(1);
    if workers <= 1 {
        let heap = score_range_into_heap(cursor, k, avg_dl);
        return heap_to_candidates(heap);
    }

    let starts = cursor.block_starts();
    let per_worker = num_blocks.div_ceil(workers);
    let ranges: Vec<(usize, usize)> = (0..workers)
        .map(|w| {
            let lo = (w * per_worker).min(num_blocks);
            let hi = ((w + 1) * per_worker).min(num_blocks);
            (lo, hi)
        })
        .filter(|(lo, hi)| lo < hi)
        .collect();

    let base: &TermCursor = cursor;
    let local_heaps: Vec<BinaryHeap<Reverse<ScoredDoc>>> = ranges
        .par_iter()
        .map(|&(lo, hi)| {
            let mut wc = base.at_block(lo, hi, &starts[lo]);
            score_range_into_heap(&mut wc, k, avg_dl)
        })
        .collect();

    let mut merged: BinaryHeap<Reverse<ScoredDoc>> = BinaryHeap::with_capacity(k + 1);
    for h in local_heaps {
        for Reverse(item) in h {
            push_topk(&mut merged, item, k);
        }
    }
    heap_to_candidates(merged)
}

struct BlockMeta {
    last_docid: DocId,
    /// idf * block_max_tf — upper bound on this term's score for any doc here.
    block_max: f32,
    start: BlockStart,
}

struct TermPlan {
    blocks: Vec<BlockMeta>,
    idf: f32,
    term_max: f32,
}

impl TermPlan {
    fn build(cursor: &TermCursor<'_>) -> Self {
        let idf = cursor.idf;
        let starts = cursor.block_starts();
        let mut blocks = Vec::with_capacity(starts.len());
        let mut term_max = 0.0f32;
        for start in &starts {
            if let Ok(h) = read_block_header(cursor.bytes, start.header_off) {
                let bmax = idf * h.block_max_tf;
                if bmax > term_max {
                    term_max = bmax;
                }
                blocks.push(BlockMeta {
                    last_docid: h.block_last_docid,
                    block_max: bmax,
                    start: *start,
                });
            }
        }
        TermPlan {
            blocks,
            idf,
            term_max,
        }
    }
}

struct BmwTerm<'a> {
    bytes: &'a [u8],
    plan: std::sync::Arc<TermPlan>,
    idf: f32,
    term_max: f32,
    cur_block: usize,
    in_block: usize,
    dec: Option<DecodedBlock>,
    exhausted: bool,
}

impl<'a> BmwTerm<'a> {
    fn new(cursor: TermCursor<'a>, plan: std::sync::Arc<TermPlan>) -> Self {
        let idf = plan.idf;
        let term_max = plan.term_max;
        let mut t = BmwTerm {
            bytes: cursor.bytes,
            plan,
            idf,
            term_max,
            cur_block: usize::MAX,
            in_block: 0,
            dec: None,
            exhausted: false,
        };
        t.decode_block(0);
        t
    }

    #[inline(always)]
    fn blocks(&self) -> &[BlockMeta] {
        &self.plan.blocks
    }

    fn is_empty(&self) -> bool {
        self.plan.blocks.is_empty()
    }

    fn decode_block(&mut self, b: usize) {
        if b >= self.blocks().len() {
            self.exhausted = true;
            self.dec = None;
            return;
        }
        if self.cur_block == b {
            return;
        }
        self.dec = decode_block_at(self.bytes, &self.plan.blocks[b].start);
        self.cur_block = b;
        self.in_block = 0;
    }

    #[inline(always)]
    fn current(&self) -> Option<DocId> {
        if self.exhausted {
            return None;
        }
        let d = self.dec.as_ref()?;
        if self.in_block < d.count {
            Some(d.doc_ids[self.in_block])
        } else {
            None
        }
    }

    #[inline(always)]
    fn block_max_here(&self) -> f32 {
        self.blocks()
            .get(self.cur_block)
            .map(|b| b.block_max)
            .unwrap_or(0.0)
    }

    fn advance_to(&mut self, target: DocId) {
        if self.exhausted {
            return;
        }
        if let Some(cur) = self.current() {
            if cur >= target {
                return;
            }
        }
        let mut b = if self.cur_block == usize::MAX {
            0
        } else {
            self.cur_block
        };
        let nblocks = self.blocks().len();
        while b < nblocks && self.blocks()[b].last_docid < target {
            b += 1;
        }
        if b >= nblocks {
            self.exhausted = true;
            self.dec = None;
            return;
        }
        self.decode_block(b);
        let Some(d) = self.dec.as_ref() else {
            self.exhausted = true;
            return;
        };
        let mut left = 0usize;
        let mut right = d.count;
        while left < right {
            let mid = left + (right - left) / 2;
            if d.doc_ids[mid] < target {
                left = mid + 1;
            } else {
                right = mid;
            }
        }
        self.in_block = left;
        if self.in_block >= d.count {
            self.advance_block();
        }
    }

    #[inline]
    fn next(&mut self) {
        if self.exhausted {
            return;
        }
        self.in_block += 1;
        if let Some(d) = self.dec.as_ref() {
            if self.in_block >= d.count {
                self.advance_block();
            }
        }
    }

    fn advance_block(&mut self) {
        let next = if self.cur_block == usize::MAX {
            0
        } else {
            self.cur_block + 1
        };
        if next >= self.blocks().len() {
            self.exhausted = true;
            self.dec = None;
        } else {
            self.decode_block(next);
        }
    }

    #[inline(always)]
    fn score_current(&self, avg_dl: f32) -> f32 {
        match self.dec.as_ref() {
            Some(d) if self.in_block < d.count => {
                self.idf * tf_component(d.tfs[self.in_block] as f32, avg_dl)
            }
            _ => 0.0,
        }
    }
}

fn decode_block_at(bytes: &[u8], start: &BlockStart) -> Option<DecodedBlock> {
    let h = read_block_header(bytes, start.header_off).ok()?;
    let count = h.count as usize;
    let mut deltas = [0u32; BLOCK_SIZE];
    bitunpack_into(
        &bytes[h.docid_payload_off..h.docid_payload_off + (h.tf_payload_off - h.docid_payload_off)],
        h.docid_bits,
        count,
        &mut deltas,
    );
    let mut block = DecodedBlock {
        doc_ids: [0u64; BLOCK_SIZE],
        tfs: [0u32; BLOCK_SIZE],
        count,
    };
    let tf_slice = &bytes[h.tf_payload_off..h.tf_payload_off + h.tf_payload_len];
    let mut tf_pos = 0usize;
    let mut prev = start.prev_doc;
    for i in 0..count {
        prev = prev.wrapping_add(deltas[i] as u64);
        block.doc_ids[i] = prev;
        block.tfs[i] = varbyte_decode(tf_slice, &mut tf_pos).saturating_add(1);
    }
    Some(block)
}

fn block_max_wand(
    cursors: Vec<TermCursor<'_>>,
    plans: &[std::sync::Arc<TermPlan>],
    k: usize,
    avg_dl: f32,
    lo: DocId,
    hi: DocId,
) -> CandidateSet {
    let mut terms: Vec<BmwTerm> = cursors
        .into_iter()
        .zip(plans.iter())
        .map(|(c, p)| BmwTerm::new(c, std::sync::Arc::clone(p)))
        .filter(|t| !t.is_empty())
        .collect();
    if terms.is_empty() {
        return CandidateSet::default();
    }
    if lo > 0 {
        for t in terms.iter_mut() {
            t.advance_to(lo);
        }
    }

    let mut heap: BinaryHeap<Reverse<ScoredDoc>> = BinaryHeap::with_capacity(k + 1);
    let mut threshold = f32::NEG_INFINITY;
    let n = terms.len();
    let mut order: Vec<usize> = (0..n).collect();

    let order_key = |terms: &[BmwTerm], ti: usize| -> u64 {
        terms[ti].current().map(|id| id as u64).unwrap_or(u64::MAX)
    };

    loop {
        for i in 1..order.len() {
            let mut j = i;
            while j > 0 && order_key(&terms, order[j - 1]) > order_key(&terms, order[j]) {
                order.swap(j - 1, j);
                j -= 1;
            }
        }
        if terms[order[0]].current().is_none() {
            break;
        }

        let mut acc = 0.0f32;
        let mut pivot_ord = None;
        for (oi, &ti) in order.iter().enumerate() {
            if terms[ti].current().is_none() {
                break;
            }
            acc += terms[ti].term_max;
            if acc >= threshold {
                pivot_ord = Some(oi);
                break;
            }
        }
        let Some(pivot_ord) = pivot_ord else { break };
        let pivot_doc = terms[order[pivot_ord]].current().unwrap();
        if pivot_doc >= hi {
            break;
        }

        if terms[order[0]].current() == Some(pivot_doc) {
            let mut block_bound = 0.0f32;
            for &ti in order.iter() {
                if terms[ti].current() == Some(pivot_doc) {
                    block_bound += terms[ti].block_max_here();
                }
            }
            if heap.len() >= k && block_bound < threshold {
                for &ti in order.iter() {
                    if terms[ti].current() == Some(pivot_doc) {
                        terms[ti].advance_to(pivot_doc + 1);
                    }
                }
                continue;
            }
            let mut score = 0.0f32;
            for ti in 0..n {
                if terms[ti].current() == Some(pivot_doc) {
                    score += terms[ti].score_current(avg_dl);
                }
            }
            for ti in 0..n {
                if terms[ti].current() == Some(pivot_doc) {
                    terms[ti].next();
                }
            }
            push_topk(
                &mut heap,
                ScoredDoc {
                    score,
                    doc_id: pivot_doc,
                },
                k,
            );
            if heap.len() >= k {
                threshold = heap.peek().map(|m| m.0.score).unwrap_or(threshold);
            }
        } else {
            for &ti in order.iter() {
                match terms[ti].current() {
                    Some(d) if d < pivot_doc => terms[ti].advance_to(pivot_doc),
                    _ => {}
                }
            }
        }
    }

    heap_to_candidates(heap)
}

struct TermCursor<'a> {
    bytes: &'a [u8],
    total_docs: usize,
    num_blocks: usize,
    block_idx: usize,
    next_block_off: usize,
    current_header: Option<BlockHeader>,
    decoded: Option<DecodedBlock>,
    prev_doc: DocId,
    pub idf: f32,
}

struct DecodedBlock {
    doc_ids: [DocId; BLOCK_SIZE],
    tfs: [u32; BLOCK_SIZE],
    count: usize,
}

#[derive(Clone, Copy)]
struct BlockStart {
    header_off: usize,
    prev_doc: DocId,
}

impl<'a> TermCursor<'a> {
    fn open(
        bytes: &'a [u8],
        base: usize,
        df: u32,
        num_docs: u32,
        _avg_dl: f32,
    ) -> Result<Self, String> {
        if base + 8 > bytes.len() {
            return Err("truncated TBM3 postings header".into());
        }
        let total = u32::from_le_bytes(bytes[base..base + 4].try_into().unwrap()) as usize;
        let num_blocks = u32::from_le_bytes(bytes[base + 4..base + 8].try_into().unwrap()) as usize;
        let start = base + 8;
        let n = num_docs.max(1) as f32;
        let idf = ((n - df as f32 + 0.5) / (df as f32 + 0.5) + 1.0).ln();

        let current_header = if num_blocks > 0 {
            Some(read_block_header(bytes, start)?)
        } else {
            None
        };
        Ok(Self {
            bytes,
            total_docs: total,
            num_blocks,
            block_idx: 0,
            next_block_off: start,
            current_header,
            decoded: None,
            prev_doc: 0,
            idf,
        })
    }

    fn reopen(&self) -> TermCursor<'a> {
        let current_header = if self.num_blocks > 0 {
            read_block_header(self.bytes, self.next_block_off).ok()
        } else {
            None
        };
        TermCursor {
            bytes: self.bytes,
            total_docs: self.total_docs,
            num_blocks: self.num_blocks,
            block_idx: 0,
            next_block_off: self.next_block_off,
            current_header,
            decoded: None,
            prev_doc: 0,
            idf: self.idf,
        }
    }

    fn block_starts(&self) -> Vec<BlockStart> {
        let mut starts = Vec::with_capacity(self.num_blocks);
        let mut off = self.next_block_off;
        let mut prev_doc: DocId = 0;
        for _ in 0..self.num_blocks {
            starts.push(BlockStart {
                header_off: off,
                prev_doc,
            });
            match read_block_header(self.bytes, off) {
                Ok(h) => {
                    prev_doc = h.block_last_docid;
                    off = h.next_block_off;
                }
                Err(_) => break,
            }
        }
        starts
    }

    fn at_block(&self, block_idx: usize, end_block: usize, start: &BlockStart) -> TermCursor<'a> {
        let current_header = if block_idx < end_block {
            read_block_header(self.bytes, start.header_off).ok()
        } else {
            None
        };
        TermCursor {
            bytes: self.bytes,
            total_docs: self.total_docs,
            num_blocks: end_block,
            block_idx,
            next_block_off: start.header_off,
            current_header,
            decoded: None,
            prev_doc: start.prev_doc,
            idf: self.idf,
        }
    }

    #[inline]
    fn has_block(&self) -> bool {
        self.block_idx < self.num_blocks
    }

    #[inline]
    fn block_count(&self) -> usize {
        self.decoded.as_ref().map(|d| d.count).unwrap_or(0)
    }

    fn decode_current_block(&mut self, deltas: &mut [u32; BLOCK_SIZE]) {
        let h = match self.current_header {
            Some(h) => h,
            None => return,
        };
        let count = h.count as usize;
        bitunpack_into(
            &self.bytes[h.docid_payload_off
                ..h.docid_payload_off + (h.tf_payload_off - h.docid_payload_off)],
            h.docid_bits,
            count,
            deltas,
        );
        let mut block = DecodedBlock {
            doc_ids: [0u64; BLOCK_SIZE],
            tfs: [0u32; BLOCK_SIZE],
            count,
        };
        let tf_slice = &self.bytes[h.tf_payload_off..h.tf_payload_off + h.tf_payload_len];
        let mut tf_pos = 0usize;
        let mut prev = self.prev_doc;
        for i in 0..count {
            prev = prev.wrapping_add(deltas[i] as u64);
            block.doc_ids[i] = prev;
            block.tfs[i] = varbyte_decode(tf_slice, &mut tf_pos).saturating_add(1);
        }
        self.decoded = Some(block);
    }

    fn advance_block(&mut self) {
        if let Some(h) = self.current_header {
            self.prev_doc = h.block_last_docid;
            self.next_block_off = h.next_block_off;
        }
        self.block_idx += 1;
        if self.block_idx < self.num_blocks {
            self.current_header = read_block_header(self.bytes, self.next_block_off).ok();
        } else {
            self.current_header = None;
        }
        self.decoded = None;
    }
}

#[derive(Clone, Copy)]
struct ScoredDoc {
    score: f32,
    doc_id: DocId,
}

impl PartialEq for ScoredDoc {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score && self.doc_id == other.doc_id
    }
}
impl Eq for ScoredDoc {}
impl Ord for ScoredDoc {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.score
            .partial_cmp(&other.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| other.doc_id.cmp(&self.doc_id))
    }
}
impl PartialOrd for ScoredDoc {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[inline]
fn push_topk(heap: &mut BinaryHeap<Reverse<ScoredDoc>>, item: ScoredDoc, k: usize) {
    if heap.len() < k {
        heap.push(Reverse(item));
        return;
    }
    if let Some(min) = heap.peek() {
        if item.cmp(&min.0) == std::cmp::Ordering::Greater {
            heap.pop();
            heap.push(Reverse(item));
        }
    }
}

fn heap_to_candidates(heap: BinaryHeap<Reverse<ScoredDoc>>) -> CandidateSet {
    let mut v: Vec<ScoredDoc> = heap.into_iter().map(|r| r.0).collect();
    v.sort_by(|a, b| b.cmp(a));
    let mut out = CandidateSet::with_capacity(v.len());
    for s in v {
        out.push(s.doc_id, s.score);
    }
    out
}

fn scores_to_topk(scores: HashMap<DocId, f32>, k: usize) -> CandidateSet {
    let mut heap: BinaryHeap<Reverse<ScoredDoc>> = BinaryHeap::with_capacity(k + 1);
    for (doc_id, score) in scores {
        push_topk(&mut heap, ScoredDoc { score, doc_id }, k);
    }
    heap_to_candidates(heap)
}

/// Rebuild an in-memory snapshot (e.g. merge table-level index from segments).
pub fn snapshot_from_tbm3(bytes: &[u8]) -> Result<Bm25Snapshot, String> {
    let view = Bm25Tbm3View::open(bytes)?;
    let mut postings: HashMap<String, Vec<(DocId, u32)>> = HashMap::new();
    let mut doc_freq: HashMap<String, u32> = HashMap::new();
    let mut doc_len: HashMap<DocId, u32> = HashMap::new();
    for i in 0..view.num_terms {
        let (term, df, off, _plen) = view.term_at(i)?;
        let term_string = term.to_string();
        doc_freq.insert(term_string.clone(), df);
        let list = view.load_term(df, off)?;
        for &(doc_id, tf) in &list.docs {
            let entry = doc_len.entry(doc_id).or_insert(0);
            *entry = entry.saturating_add(tf);
        }
        postings.insert(term_string, list.docs);
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
        let snap =
            Bm25Snapshot::from_documents([(1u64, "Nikola Tesla motor"), (2, "wireless power")]);
        let bytes = encode_tbm3(&snap);
        let view = Bm25Tbm3View::open(&bytes).unwrap();
        let hits = view.search("Nikola motor", 5);
        assert!(!hits.is_empty());
    }

    #[test]
    fn range_partitioned_bmw_matches_full() {
        let mut docs: Vec<(u64, String)> = Vec::new();
        for d in 0..3000u64 {
            docs.push((
                d,
                format!("common term{} rare{} body{}", d % 11, d % 257, d),
            ));
        }
        let snap = Bm25Snapshot::from_documents(docs.iter().map(|(i, t)| (*i, t.as_str())));
        let bytes = encode_tbm3(&snap);
        let view = Bm25Tbm3View::open(&bytes).unwrap();
        let k = 15;
        let query = "common term3 rare40";

        let plans_for = |c: &[TermCursor]| -> Vec<std::sync::Arc<TermPlan>> {
            c.iter()
                .map(|x| std::sync::Arc::new(TermPlan::build(x)))
                .collect()
        };
        let full = {
            let mut c = view.cursors_for(query);
            c.sort_by_key(|x| x.total_docs);
            let plans = plans_for(&c);
            block_max_wand(c, &plans, k, view.avg_dl, 0, DocId::MAX)
        };
        // Four disjoint shards, merged like the parallel path does.
        let bounds = [0u64, 750, 1500, 2250, DocId::MAX];
        let mut merged: BinaryHeap<Reverse<ScoredDoc>> = BinaryHeap::with_capacity(k + 1);
        for w in 0..4 {
            let mut c = view.cursors_for(query);
            c.sort_by_key(|x| x.total_docs);
            let plans = plans_for(&c);
            let set = block_max_wand(c, &plans, k, view.avg_dl, bounds[w], bounds[w + 1]);
            for (i, &doc_id) in set.ids.iter().enumerate() {
                push_topk(
                    &mut merged,
                    ScoredDoc {
                        score: set.scores[i],
                        doc_id,
                    },
                    k,
                );
            }
        }
        let sharded = heap_to_candidates(merged);
        let brute = view.brute_search(query, k);

        let key = |c: &CandidateSet| -> Vec<u64> { c.ids.clone() };
        assert_eq!(key(&full), key(&brute), "full BMW must equal brute force");
        assert_eq!(
            key(&sharded),
            key(&brute),
            "sharded BMW must equal brute force"
        );
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
        let mut wand_pairs: Vec<_> = wand.ids.into_iter().zip(wand.scores).collect();
        let mut brute_pairs: Vec<_> = brute.ids.into_iter().zip(brute.scores).collect();
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
        postings.insert("test".to_string(), vec![(3, 2), (1, 1), (2, 1)]);
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
            let start = if bi == 0 {
                0
            } else {
                list.block_ends[bi - 1] as usize
            };
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

    #[test]
    fn varbyte_roundtrip() {
        let vals: Vec<u32> = vec![0, 1, 127, 128, 16383, 16384, 1 << 21, u32::MAX];
        let mut buf = Vec::new();
        for &v in &vals {
            varbyte_encode(v, &mut buf);
        }
        let mut pos = 0;
        for &expected in &vals {
            assert_eq!(varbyte_decode(&buf, &mut pos), expected);
        }
    }

    #[test]
    fn bitpack_roundtrip_random() {
        let vals: Vec<u32> = (0..200u32).map(|i| (i * 7) % 1024).collect();
        let bits = bits_needed(*vals.iter().max().unwrap());
        let mut payload = Vec::new();
        bitpack(&vals, bits, &mut payload);
        let mut out = [0u32; BLOCK_SIZE];
        // Decode in chunks of 128 to mirror runtime use.
        for chunk_start in (0..vals.len()).step_by(BLOCK_SIZE) {
            let count = (vals.len() - chunk_start).min(BLOCK_SIZE);
            let mut chunk_payload = Vec::new();
            bitpack(
                &vals[chunk_start..chunk_start + count],
                bits,
                &mut chunk_payload,
            );
            bitunpack_into(&chunk_payload, bits, count, &mut out);
            for i in 0..count {
                assert_eq!(out[i], vals[chunk_start + i]);
            }
        }
    }

    #[test]
    fn search_matches_brute_force_topk() {
        // Larger corpus to exercise MaxScore + heap path.
        let mut docs = Vec::new();
        for i in 0..2000u64 {
            docs.push((
                i,
                format!(
                    "alpha{} beta{} gamma{} doc text content {}",
                    i % 7,
                    i % 13,
                    i % 5,
                    i
                ),
            ));
        }
        let snap = Bm25Snapshot::from_documents(docs);
        let bytes = encode_tbm3(&snap);
        let view = Bm25Tbm3View::open(&bytes).unwrap();
        for q in [
            "alpha3 doc",
            "beta7 gamma2 doc text",
            "alpha0 beta0 gamma0 content",
            "doc",
        ] {
            let wand = view.search(q, 10);
            let brute = view.brute_search(q, 10);
            let mut a: Vec<(DocId, f32)> = wand.ids.into_iter().zip(wand.scores).collect();
            let mut b: Vec<(DocId, f32)> = brute.ids.into_iter().zip(brute.scores).collect();
            a.sort_by(|x, y| {
                y.1.partial_cmp(&x.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| x.0.cmp(&y.0))
            });
            b.sort_by(|x, y| {
                y.1.partial_cmp(&x.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| x.0.cmp(&y.0))
            });
            assert_eq!(a, b, "mismatch on query {q}");
        }
    }

    #[test]
    fn parallel_single_term_matches_sequential() {
        let n: u64 = 270_000;
        let mut docs = Vec::with_capacity(n as usize);
        for i in 0..n {
            docs.push((i, format!("common rare{} body{}", i % 100, i)));
        }
        let snap = Bm25Snapshot::from_documents(docs);
        let bytes = encode_tbm3(&snap);
        let view = Bm25Tbm3View::open(&bytes).unwrap();

        let parallel = view.search("common", 10);
        let brute = view.brute_search("common", 10);

        let sort_key = |a: &(DocId, f32), b: &(DocId, f32)| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        };
        let mut p: Vec<(DocId, f32)> = parallel.ids.into_iter().zip(parallel.scores).collect();
        let mut b: Vec<(DocId, f32)> = brute.ids.into_iter().zip(brute.scores).collect();
        p.sort_by(sort_key);
        b.sort_by(sort_key);
        assert_eq!(p, b, "parallel single-term result must match brute force");
    }

    #[test]
    fn bmw_matches_brute_force_random() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut rng = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let vocab: Vec<String> = (0..300).map(|i| format!("v{i}")).collect();
        let mut docs: Vec<(u64, String)> = Vec::new();
        for d in 0..6000u64 {
            let len = 8 + (rng() % 60) as usize;
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
        let snap = Bm25Snapshot::from_documents(docs.iter().map(|(i, t)| (*i, t.as_str())));
        let bytes = encode_tbm3(&snap);
        let view = Bm25Tbm3View::open(&bytes).unwrap();

        let sort_key = |a: &(DocId, f32), b: &(DocId, f32)| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        };
        for _ in 0..150 {
            let qlen = 2 + (rng() % 8) as usize; // multi-term to hit BMW
            let query: String = (0..qlen)
                .map(|_| {
                    let r = (rng() % vocab.len() as u64) as usize;
                    let idx = (r * r) / vocab.len();
                    vocab[idx.min(vocab.len() - 1)].clone()
                })
                .collect::<Vec<_>>()
                .join(" ");
            let k = 1 + (rng() % 25) as usize;
            let mut got: Vec<(DocId, f32)> = {
                let c = view.search(&query, k);
                c.ids.into_iter().zip(c.scores).collect()
            };
            let mut want: Vec<(DocId, f32)> = {
                let c = view.brute_search(&query, k);
                c.ids.into_iter().zip(c.scores).collect()
            };
            got.sort_by(sort_key);
            want.sort_by(sort_key);
            assert_eq!(
                got.iter().map(|x| x.0).collect::<Vec<_>>(),
                want.iter().map(|x| x.0).collect::<Vec<_>>(),
                "id mismatch query={query:?} k={k}"
            );
            for (i, (_, ws)) in want.iter().enumerate() {
                assert!(
                    (got[i].1 - ws).abs() < 1e-3,
                    "score mismatch query={query:?}"
                );
            }
        }
    }

    #[test]
    fn rejects_old_version() {
        // Build a synthetic header with old VERSION=1 and ensure open() refuses.
        let mut buf = Vec::new();
        buf.extend_from_slice(TBM3_MAGIC);
        buf.push(1u8);
        buf.extend_from_slice(&[0u8; 3]);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0f32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        assert!(Bm25Tbm3View::open(&buf).is_err());
    }
}
