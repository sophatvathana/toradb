//! BM25 on-disk format `TBM3`: sorted term dictionary + posting lists (mmap-friendly).

use std::collections::HashMap;
use std::path::Path;

use memmap2::Mmap;
use toradb_core::{CandidateSet, DocId};

use super::bm25::{tokenize, B, K1, Bm25Snapshot};

pub fn decode_snapshot(bytes: &[u8]) -> Result<Bm25Snapshot, String> {
    snapshot_from_tbm3(bytes)
}

pub fn write_snapshot_file(path: &Path, snap: &Bm25Snapshot) -> Result<(), String> {
    write_tbm3_file(path, snap)
}

pub const TBM3_MAGIC: &[u8; 4] = b"TBM3";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 20;

pub fn encode_tbm3(snap: &Bm25Snapshot) -> Vec<u8> {
    let mut terms: Vec<&String> = snap.postings.keys().collect();
    terms.sort();
    let num_terms = terms.len() as u32;

    let mut dict = Vec::new();
    let mut postings_blob = Vec::new();
    for term in &terms {
        let posts = &snap.postings[*term];
        let offset = postings_blob.len() as u32;
        postings_blob.extend_from_slice(&(posts.len() as u32).to_le_bytes());
        for &(doc_id, tf) in posts {
            postings_blob.extend_from_slice(&doc_id.to_le_bytes());
            postings_blob.extend_from_slice(&tf.to_le_bytes());
        }
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

    fn read_postings(&self, rel_off: u32, _plen: u32) -> Vec<(DocId, u32)> {
        let base = self.postings_start + rel_off as usize;
        if base + 4 > self.bytes.len() {
            return Vec::new();
        }
        let count = u32::from_le_bytes(self.bytes[base..base + 4].try_into().unwrap()) as usize;
        let mut pos = base + 4;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            if pos + 12 > self.bytes.len() {
                break;
            }
            let doc_id = u64::from_le_bytes(self.bytes[pos..pos + 8].try_into().unwrap());
            let tf = u32::from_le_bytes(self.bytes[pos + 8..pos + 12].try_into().unwrap());
            pos += 12;
            out.push((doc_id, tf));
        }
        out
    }

    pub fn search(&self, query: &str, k: usize) -> CandidateSet {
        let mut scores: HashMap<DocId, f32> = HashMap::new();
        let n = self.num_docs.max(1) as f32;
        for term in tokenize(query) {
            let Some((df, rel_off, _plen)) = self.find_term_entry(&term) else {
                continue;
            };
            let idf = ((n - df as f32 + 0.5) / (df as f32 + 0.5) + 1.0).ln();
            let base = self.postings_start + rel_off as usize;
            if base + 4 > self.bytes.len() {
                continue;
            }
            let count = u32::from_le_bytes(self.bytes[base..base + 4].try_into().unwrap()) as usize;
            let mut pos = base + 4;
            for _ in 0..count {
                if pos + 12 > self.bytes.len() {
                    break;
                }
                let doc_id = u64::from_le_bytes(self.bytes[pos..pos + 8].try_into().unwrap());
                let tf = u32::from_le_bytes(self.bytes[pos + 8..pos + 12].try_into().unwrap());
                pos += 12;
                let dl = tf as f32;
                let denom = tf as f32 + K1 * (1.0 - B + B * dl / self.avg_dl.max(1.0));
                let score = idf * (tf as f32 * (K1 + 1.0)) / denom;
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

/// Rebuild an in-memory snapshot (e.g. merge table-level index from segments).
pub fn snapshot_from_tbm3(bytes: &[u8]) -> Result<Bm25Snapshot, String> {
    let view = Bm25Tbm3View::open(bytes)?;
    let mut postings: HashMap<String, Vec<(DocId, u32)>> = HashMap::new();
    let mut doc_freq: HashMap<String, u32> = HashMap::new();
    let mut doc_len: HashMap<DocId, u32> = HashMap::new();
    for i in 0..view.num_terms {
        let (term, df, off, plen) = view.term_at(i)?;
        doc_freq.insert(term.to_string(), df);
        let posts = view.read_postings(off, plen);
        for &(doc_id, tf) in &posts {
            let entry = doc_len.entry(doc_id).or_insert(0);
            *entry = entry.saturating_add(tf);
        }
        postings.insert(term.to_string(), posts);
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
}
