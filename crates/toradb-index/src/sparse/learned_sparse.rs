use std::collections::HashMap;
use std::path::Path;

use toradb_core::DocId;

use crate::sparse::learned::{SparseInterner, SparseSnapshot};

pub const LSP1_MAGIC: &[u8; 4] = b"LSP1";
pub const LSP1_VERSION: u8 = 1;

fn varbyte_encode_u64(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push(((v as u8) & 0x7F) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

#[inline(always)]
fn varbyte_decode_u64(bytes: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut v: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        let b = *bytes.get(*pos).ok_or("varbyte: unexpected end")?;
        *pos += 1;
        v |= ((b & 0x7F) as u64) << shift;
        if b < 0x80 {
            return Ok(v);
        }
        shift += 7;
        if shift >= 64 {
            return Err("varbyte: overflow".into());
        }
    }
}

pub fn encode_lsp1(snap: &SparseSnapshot) -> Vec<u8> {
    let terms = snap.interner.terms();
    let num_terms = terms.len();

    let mut blob: Vec<u8> = Vec::new();
    let mut offsets: Vec<(u32, u32)> = Vec::with_capacity(num_terms);
    for id in 0..num_terms as u32 {
        let start = blob.len() as u32;
        let posts = snap.postings.get(&id);
        let count = posts.map(|p| p.len()).unwrap_or(0) as u32;
        blob.extend_from_slice(&count.to_le_bytes());
        if let Some(posts) = posts {
            let mut sorted: Vec<(DocId, f32)> = posts.clone();
            if sorted.windows(2).any(|w| w[0].0 > w[1].0) {
                sorted.sort_unstable_by_key(|&(d, _)| d);
            }
            let mut prev: u64 = 0;
            for (doc, w) in sorted {
                let delta = doc - prev;
                prev = doc;
                varbyte_encode_u64(delta, &mut blob);
                blob.extend_from_slice(&w.to_le_bytes());
            }
        }
        let len = blob.len() as u32 - start;
        offsets.push((start, len));
    }

    let mut dict: Vec<u8> = Vec::new();
    for (id, term) in terms.iter().enumerate() {
        let bytes = term.as_bytes();
        let tlen = bytes.len() as u16;
        dict.extend_from_slice(&tlen.to_le_bytes());
        dict.extend_from_slice(bytes);
        let (off, len) = offsets[id];
        dict.extend_from_slice(&off.to_le_bytes());
        dict.extend_from_slice(&len.to_le_bytes());
    }

    let mut out: Vec<u8> = Vec::with_capacity(12 + dict.len() + blob.len());
    out.extend_from_slice(LSP1_MAGIC);
    out.push(LSP1_VERSION);
    out.extend_from_slice(&[0u8; 3]); // pad
    out.extend_from_slice(&snap.num_docs.to_le_bytes());
    out.extend_from_slice(&(num_terms as u32).to_le_bytes());
    out.extend_from_slice(&dict);
    out.extend_from_slice(&blob);
    out
}

pub fn write_lsp1_file(path: &Path, snap: &SparseSnapshot) -> Result<(), String> {
    let bytes = encode_lsp1(snap);
    let tmp = path.with_extension("lsp1.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn read_lsp1_version(path: &Path) -> Option<u8> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 5];
    f.read_exact(&mut buf).ok()?;
    if &buf[..4] != LSP1_MAGIC {
        return None;
    }
    Some(buf[4])
}

pub fn snapshot_from_lsp1(bytes: &[u8]) -> Result<SparseSnapshot, String> {
    if bytes.len() < 12 {
        return Err("LSP1: file too small".into());
    }
    if &bytes[..4] != LSP1_MAGIC {
        return Err("LSP1: bad magic".into());
    }
    let version = bytes[4];
    if version != LSP1_VERSION {
        return Err(format!("LSP1: unsupported version {version}"));
    }
    let num_docs = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let num_terms = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;

    // The header is 16 bytes (magic 4 + ver 1 + pad 3 + num_docs 4 + num_terms 4).
    let mut pos = 16usize;
    let mut terms: Vec<String> = Vec::with_capacity(num_terms);
    let mut term_offsets: Vec<(u32, u32)> = Vec::with_capacity(num_terms);
    for _ in 0..num_terms {
        if pos + 2 > bytes.len() {
            return Err("LSP1: truncated dict (len)".into());
        }
        let tlen = u16::from_le_bytes(bytes[pos..pos + 2].try_into().unwrap()) as usize;
        pos += 2;
        if pos + tlen + 8 > bytes.len() {
            return Err("LSP1: truncated dict (term)".into());
        }
        let term = std::str::from_utf8(&bytes[pos..pos + tlen])
            .map_err(|_| "LSP1: invalid utf8 term")?
            .to_string();
        pos += tlen;
        let off = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
        pos += 4;
        terms.push(term);
        term_offsets.push((off, len));
    }

    let blob_start = pos;
    let blob = &bytes[blob_start..];
    let mut postings: HashMap<u32, Vec<(DocId, f32)>> = HashMap::with_capacity(num_terms);
    for (id, (off, len)) in term_offsets.iter().enumerate() {
        let off = *off as usize;
        let len = *len as usize;
        if off + len > blob.len() {
            return Err("LSP1: postings out of range".into());
        }
        let seg = &blob[off..off + len];
        let mut p = 0usize;
        if seg.len() < 4 {
            return Err("LSP1: postings count missing".into());
        }
        let count = u32::from_le_bytes(seg[0..4].try_into().unwrap()) as usize;
        p += 4;
        if count == 0 {
            continue;
        }
        let mut posts: Vec<(DocId, f32)> = Vec::with_capacity(count);
        let mut prev: u64 = 0;
        for _ in 0..count {
            let delta = varbyte_decode_u64(seg, &mut p)?;
            let doc = prev + delta;
            prev = doc;
            if p + 4 > seg.len() {
                return Err("LSP1: postings weight missing".into());
            }
            let w = f32::from_le_bytes(seg[p..p + 4].try_into().unwrap());
            p += 4;
            posts.push((doc, w));
        }
        postings.insert(id as u32, posts);
    }

    let mut interner = SparseInterner::default();
    for term in &terms {
        interner.intern(term);
    }

    Ok(SparseSnapshot {
        interner,
        postings,
        num_docs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sparse::learned::{SparseProfile, SparseWeightedIndex};

    fn map(pairs: &[(&str, f32)]) -> HashMap<String, f32> {
        pairs.iter().map(|&(t, w)| (t.to_string(), w)).collect()
    }

    #[test]
    fn lsp1_roundtrip_preserves_ranking() {
        let mut idx = SparseWeightedIndex::default();
        idx.add_document(0, &map(&[("alpha", 1.0), ("beta", 2.0)]));
        idx.add_document(5, &map(&[("beta", 0.5), ("gamma", 3.0)]));
        idx.add_document(9, &map(&[("alpha", 4.0), ("gamma", 1.0)]));
        let snap = idx.snapshot();

        let bytes = encode_lsp1(&snap);
        let decoded = snapshot_from_lsp1(&bytes).unwrap();
        let restored = SparseWeightedIndex::from_snapshot(decoded);

        for q in [
            map(&[("alpha", 1.0)]),
            map(&[("beta", 1.0), ("gamma", 1.0)]),
            map(&[("gamma", 2.0)]),
        ] {
            let a = idx.search_text(&q, 10, SparseProfile::Splade);
            let b = restored.search_text(&q, 10, SparseProfile::Splade);
            assert_eq!(a.ids, b.ids, "ids differ after roundtrip");
            for (sa, sb) in a.scores.iter().zip(b.scores.iter()) {
                assert!((sa - sb).abs() < 1e-6, "scores differ: {sa} vs {sb}");
            }
        }
    }

    #[test]
    fn lsp1_handles_large_doc_ids() {
        let mut idx = SparseWeightedIndex::default();
        idx.add_document(0, &map(&[("x", 1.0)]));
        idx.add_document(5_000_000_000, &map(&[("x", 2.0)])); // > u32::MAX
        let snap = idx.snapshot();
        let decoded = snapshot_from_lsp1(&encode_lsp1(&snap)).unwrap();
        let restored = SparseWeightedIndex::from_snapshot(decoded);
        let got = restored.search_text(&map(&[("x", 1.0)]), 5, SparseProfile::Splade);
        assert_eq!(got.ids, vec![5_000_000_000, 0]);
    }

    #[test]
    fn lsp1_rejects_bad_magic() {
        let mut bad = vec![0u8; 32];
        bad[0] = b'X';
        assert!(snapshot_from_lsp1(&bad).is_err());
    }

    #[test]
    fn lsp1_rejects_bad_version() {
        let mut idx = SparseWeightedIndex::default();
        idx.add_document(0, &map(&[("x", 1.0)]));
        let mut bytes = encode_lsp1(&idx.snapshot());
        bytes[4] = 99;
        assert!(snapshot_from_lsp1(&bytes).is_err());
    }
}
