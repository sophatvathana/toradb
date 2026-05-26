//! Per-segment BM25 term lexicon (`TLEX`) for query-time segment pruning.

use std::path::Path;

pub const LEXICON_MAGIC: &[u8; 4] = b"TLEX";

/// Build sorted unique terms from a BM25 snapshot's posting keys.
pub fn terms_from_posting_keys<'a>(terms: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut v: Vec<String> = terms.map(|s| s.to_string()).collect();
    v.sort();
    v.dedup();
    v
}

pub fn encode_lexicon(terms: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(LEXICON_MAGIC);
    out.extend_from_slice(&(terms.len() as u32).to_le_bytes());
    for term in terms {
        let bytes = term.as_bytes();
        let len = bytes.len().min(u16::MAX as usize) as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&bytes[..len as usize]);
    }
    out
}

pub fn write_lexicon_file(path: &Path, terms: &[String]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = encode_lexicon(terms);
    let tmp = path.with_extension("lex.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Memory-mapped or in-memory lexicon for binary search.
pub struct Bm25LexiconView<'a> {
    pub terms: Vec<&'a str>,
}

impl<'a> Bm25LexiconView<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        if bytes.len() < 8 || &bytes[..4] != LEXICON_MAGIC {
            return Err("invalid bm25 lexicon magic".into());
        }
        let n = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let mut terms = Vec::with_capacity(n);
        let mut pos = 8usize;
        for _ in 0..n {
            if pos + 2 > bytes.len() {
                return Err("truncated lexicon".into());
            }
            let len = u16::from_le_bytes(bytes[pos..pos + 2].try_into().unwrap()) as usize;
            pos += 2;
            if pos + len > bytes.len() {
                return Err("truncated lexicon term".into());
            }
            let s = std::str::from_utf8(&bytes[pos..pos + len]).map_err(|e| e.to_string())?;
            terms.push(s);
            pos += len;
        }
        Ok(Self { terms })
    }

    pub fn contains_term(&self, term: &str) -> bool {
        self.terms.binary_search(&term).is_ok()
    }

    pub fn intersects_query_terms<'b, I: Iterator<Item = &'b str>>(&self, query_terms: I) -> bool {
        for t in query_terms {
            if self.contains_term(t) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexicon_roundtrip_and_search() {
        let terms = vec!["apple".into(), "banana".into(), "zebra".into()];
        let bytes = encode_lexicon(&terms);
        let view = Bm25LexiconView::parse(&bytes).unwrap();
        assert!(view.contains_term("banana"));
        assert!(!view.contains_term("cherry"));
        assert!(view.intersects_query_terms(["foo", "zebra"].iter().copied()));
    }
}
