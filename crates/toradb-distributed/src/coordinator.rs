use toradb_core::{CandidateSet, DocId};

use crate::config::ClusterConfig;
use crate::protocol::{Request, Response};
use crate::rpc;

pub struct Coordinator<'a> {
    config: &'a ClusterConfig,
}

impl<'a> Coordinator<'a> {
    pub fn new(config: &'a ClusterConfig) -> Self {
        Self { config }
    }

    pub fn search_segments(
        &self,
        table: &str,
        query: &str,
        k: usize,
        segments: &[u32],
    ) -> Result<CandidateSet, String> {
        let mut merged = CandidateSet::with_capacity(k.saturating_mul(segments.len()));
        for &seg in segments {
            let Some(worker) = self.config.worker_for_segment(seg) else {
                continue;
            };
            let req = Request::SegmentSearch {
                table: table.to_string(),
                segment: seg,
                query: query.to_string(),
                k: k as u32,
            };
            let resp = rpc::call(&worker.addr, &req)?;
            match resp {
                Response::Ok {
                    candidates: Some(mut local),
                    ..
                } => {
                    for (i, id) in local.ids.iter().enumerate() {
                        let score = local.scores[i];
                        if merged.len() >= k.saturating_mul(4) {
                            break;
                        }
                        merged.push(*id, score);
                    }
                }
                Response::Error { message } => return Err(message),
                Response::Ok { candidates: None, .. } => {}
            }
        }
        merged.sort_by_score(true);
        merged.truncate(k);
        Ok(merged)
    }

    pub fn node_status(&self) -> Vec<(String, String, bool)> {
        self.config
            .workers
            .iter()
            .map(|w| {
                let healthy = matches!(
                    rpc::call(&w.addr, &Request::Health),
                    Ok(Response::Ok { .. })
                );
                (w.id.clone(), w.addr.clone(), healthy)
            })
            .collect()
    }
}

/// Merge helper used by coordinator and tests.
pub fn merge_candidate_sets(mut acc: CandidateSet, other: CandidateSet, cap: usize) -> CandidateSet {
    let mut scores: std::collections::HashMap<DocId, f32> = acc
        .ids
        .iter()
        .zip(acc.scores.iter())
        .map(|(&id, &s)| (id, s))
        .collect();
    for (i, id) in other.ids.iter().enumerate() {
        let s = other.scores[i];
        scores
            .entry(*id)
            .and_modify(|e| *e = e.max(s))
            .or_insert(s);
    }
    let mut ranked: Vec<(DocId, f32)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(cap);
    acc.ids.clear();
    acc.scores.clear();
    for (id, s) in ranked {
        acc.push(id, s);
    }
    acc
}
