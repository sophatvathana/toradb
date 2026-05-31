//! Table-level BM25 routing index (`TBRT`): term → segment indices.

use std::path::Path;

pub const ROUTE_MAGIC: &[u8; 4] = b"TBRT";

#[derive(Debug, Clone)]
pub struct RouteTermEntry {
    pub term: String,
    pub segments: Vec<u32>,
}

pub fn encode_route(entries: &[RouteTermEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(ROUTE_MAGIC);
    out.push(1);
    out.extend_from_slice(&[0u8; 3]);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        let tb = e.term.as_bytes();
        let tlen = tb.len().min(u16::MAX as usize) as u16;
        out.extend_from_slice(&tlen.to_le_bytes());
        out.extend_from_slice(&tb[..tlen as usize]);
        out.extend_from_slice(&(e.segments.len() as u32).to_le_bytes());
        for &seg in &e.segments {
            out.extend_from_slice(&seg.to_le_bytes());
        }
    }
    out
}

pub fn write_route_file(path: &Path, entries: &[RouteTermEntry]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = encode_route(entries);
    let tmp = path.with_extension("route.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub struct Bm25RouteView<'a> {
    bytes: &'a [u8],
    num_terms: u32,
    dict_start: usize,
}

impl<'a> Bm25RouteView<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Self, String> {
        if bytes.len() < 12 || &bytes[..4] != ROUTE_MAGIC {
            return Err("invalid TBRT magic".into());
        }
        let num_terms = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        Ok(Self {
            bytes,
            num_terms,
            dict_start: 12,
        })
    }

    fn term_at(&self, index: u32) -> Result<(&str, Vec<u32>), String> {
        let read_u16 = |b: &[u8], p: usize| -> Result<usize, String> {
            b.get(p..p + 2)
                .ok_or_else(|| "truncated route".to_string())
                .map(|s| u16::from_le_bytes(s.try_into().unwrap()) as usize)
        };
        let read_u32 = |b: &[u8], p: usize| -> Result<usize, String> {
            b.get(p..p + 4)
                .ok_or_else(|| "truncated route".to_string())
                .map(|s| u32::from_le_bytes(s.try_into().unwrap()) as usize)
        };
        let mut pos = self.dict_start;
        for _ in 0..index {
            let tlen = read_u16(self.bytes, pos)?;
            pos += 2 + tlen;
            let nseg = read_u32(self.bytes, pos)?;
            pos += 4 + nseg * 4;
        }
        let tlen = read_u16(self.bytes, pos)?;
        pos += 2;
        let t = self
            .bytes
            .get(pos..pos + tlen)
            .ok_or_else(|| "truncated route term".to_string())
            .and_then(|s| std::str::from_utf8(s).map_err(|e| e.to_string()))?;
        pos += tlen;
        let nseg = read_u32(self.bytes, pos)?;
        pos += 4;
        let mut segs = Vec::with_capacity(nseg);
        for _ in 0..nseg {
            segs.push(read_u32(self.bytes, pos)? as u32);
            pos += 4;
        }
        Ok((t, segs))
    }

    fn find_term(&self, term: &str) -> Option<Vec<u32>> {
        let mut lo = 0u32;
        let mut hi = self.num_terms;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let (t, segs) = self.term_at(mid).ok()?;
            match t.cmp(term) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(segs.to_vec()),
            }
        }
        None
    }

    /// Union of segment indices that may contain any query term.
    pub fn segments_for_query<'b, I: Iterator<Item = &'b str>>(
        &self,
        query_terms: I,
    ) -> Vec<u32> {
        let mut set: Vec<u32> = Vec::new();
        for term in query_terms {
            if let Some(segs) = self.find_term(term) {
                for s in segs {
                    if !set.contains(&s) {
                        set.push(s);
                    }
                }
            }
        }
        set.sort_unstable();
        set
    }
}

pub fn merge_lexicons_into_route(
    segment_lexicons: impl Iterator<Item = (u32, Vec<String>)>,
) -> Vec<RouteTermEntry> {
    let mut map: std::collections::BTreeMap<String, Vec<u32>> = std::collections::BTreeMap::new();
    for (seg_idx, terms) in segment_lexicons {
        for term in terms {
            map.entry(term).or_default().push(seg_idx);
        }
    }
    for segs in map.values_mut() {
        segs.sort_unstable();
        segs.dedup();
    }
    map.into_iter()
        .map(|(term, segments)| RouteTermEntry { term, segments })
        .collect()
}
