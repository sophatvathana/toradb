//! TurboQuant search: ADC-scored top-K over a `TurboQuantSnapshot`, with an
//! optional full-precision re-rank pulled from a `VectorSnapshot` sidecar.

use std::collections::BinaryHeap;

use toradb_core::{CandidateSet, DocId};
use toradb_simd::dot_f32;

use crate::dense::hnsw_index::HnswIndex;
use crate::dense::turboquant_codec::TurboQuantSnapshot;
use crate::dense::vector_codec::VectorSnapshot;

#[derive(Clone, Copy)]
struct Scored(f32, DocId);
impl PartialEq for Scored {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for Scored {}
impl Ord for Scored {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Min-heap on score (we want to evict smallest at the top).
        other
            .0
            .partial_cmp(&self.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}
impl PartialOrd for Scored {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Brute-force top-K over the snapshot using ADC scoring.
pub fn adc_topk(snap: &TurboQuantSnapshot, query: &[f32], k: usize) -> CandidateSet {
    if snap.is_empty() || k == 0 {
        return CandidateSet::default();
    }
    let qrot = snap.rotate_query(query);
    let mut heap: BinaryHeap<Scored> = BinaryHeap::with_capacity(k + 1);
    for i in 0..snap.len() {
        let s = snap.adc_dot(&qrot, i);
        let id = snap.ids[i];
        if heap.len() < k {
            heap.push(Scored(s, id));
        } else if let Some(top) = heap.peek() {
            if s > top.0 {
                heap.pop();
                heap.push(Scored(s, id));
            }
        }
    }
    let mut scored: Vec<Scored> = heap.into_vec();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = CandidateSet::with_capacity(scored.len());
    for Scored(s, id) in scored {
        out.push(id, s);
    }
    out
}

/// HNSW-guided top-K: walk the graph using ADC scores, then optionally re-rank
/// the frontier with full-precision dot from `full` if present.
pub fn hnsw_adc_search(
    graph: &HnswIndex,
    snap: &TurboQuantSnapshot,
    full: Option<&VectorSnapshot>,
    query: &[f32],
    k: usize,
    rerank_factor: usize,
) -> CandidateSet {
    if graph.len() == 0 || k == 0 {
        return CandidateSet::default();
    }
    let qrot = snap.rotate_query(query);

    let mut id_to_snap: std::collections::HashMap<DocId, usize> =
        std::collections::HashMap::with_capacity(snap.len());
    for (i, &id) in snap.ids.iter().enumerate() {
        id_to_snap.insert(id, i);
    }
    let node_to_snap: Vec<Option<usize>> = graph
        .ids()
        .iter()
        .map(|id| id_to_snap.get(id).copied())
        .collect();

    let oversample = (k * rerank_factor.max(1)).max(k);
    let approx = graph.search_with(oversample, &|i| match node_to_snap[i] {
        Some(si) => snap.adc_dot(&qrot, si),
        None => f32::NEG_INFINITY,
    });

    let Some(full) = full else {
        let mut out = CandidateSet::with_capacity(k.min(approx.len()));
        for i in 0..k.min(approx.len()) {
            out.push(approx.ids[i], approx.scores[i]);
        }
        return out;
    };

    let full_map = full.to_map();
    let mut scored: Vec<(DocId, f32)> = Vec::with_capacity(approx.len());
    for i in 0..approx.len() {
        let id = approx.ids[i];
        let approx_score = approx.scores[i];
        let s = match full_map.get(&id) {
            Some(v) => dot_f32(query, v),
            None => approx_score,
        };
        scored.push((id, s));
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    let mut out = CandidateSet::with_capacity(scored.len());
    for (id, s) in scored {
        out.push(id, s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dense::turboquant_codec::TqMode;

    fn make_corpus(n: usize, dim: usize) -> Vec<(DocId, Vec<f32>)> {
        (0..n)
            .map(|i| {
                let v: Vec<f32> = (0..dim)
                    .map(|j| ((i * 31 + j * 7) as f32 * 0.013).sin())
                    .collect();
                (i as u64, v)
            })
            .collect()
    }

    #[test]
    fn adc_topk_recovers_self_query_in_top5() {
        let pairs = make_corpus(128, 128);
        let snap = TurboQuantSnapshot::from_pairs(&pairs, TqMode::Ip, 3, 0xABCD, 0xF00D).unwrap();
        let query: Vec<f32> = pairs[42].1.clone();
        let got = adc_topk(&snap, &query, 5);
        assert!(got.ids.contains(&42), "got top5={:?}", got.ids);
    }

    #[test]
    fn hnsw_adc_rerank_matches_full_precision() {
        let pairs = make_corpus(64, 128);
        let snap = TurboQuantSnapshot::from_pairs(&pairs, TqMode::Mse, 4, 1, 0).unwrap();
        let full = VectorSnapshot::from_pairs(128, &pairs).unwrap();
        let ids: Vec<DocId> = pairs.iter().map(|(id, _)| *id).collect();
        let vecs: Vec<Vec<f32>> = pairs.iter().map(|(_, v)| v.clone()).collect();
        let graph = HnswIndex::build(ids.clone(), vecs.clone()).unwrap();
        let query: Vec<f32> = (0..128).map(|j| ((j as f32) * 0.041).cos()).collect();

        // Ground truth by exhaustive full-precision IP scan.
        let mut truth: Vec<(DocId, f32)> = pairs
            .iter()
            .map(|(id, v)| (*id, dot_f32(&query, v)))
            .collect();
        truth.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let truth_top3: Vec<DocId> = truth.iter().take(3).map(|(id, _)| *id).collect();

        let got = hnsw_adc_search(&graph, &snap, Some(&full), &query, 3, 8);
        // Re-ranked top-1 must equal the brute-force IP top-1.
        assert_eq!(
            got.ids[0], truth_top3[0],
            "rerank top1 disagrees with brute-force; got {:?} truth {:?}",
            got.ids, truth_top3
        );
        // Top-3 should overlap by at least 2.
        let overlap = got.ids.iter().filter(|id| truth_top3.contains(id)).count();
        assert!(
            overlap >= 2,
            "top3 overlap {overlap}: got {:?} truth {:?}",
            got.ids,
            truth_top3
        );
    }
}
