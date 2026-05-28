use toradb_core::CandidateSet;

use crate::config::ClusterConfig;
use crate::coordinator::Coordinator;
use crate::protocol::{Request, Response};
use crate::rpc;

#[derive(Debug)]
pub struct ClusterClient {
    config: ClusterConfig,
}

impl ClusterClient {
    pub fn new(config: ClusterConfig) -> Self {
        Self { config }
    }

    pub fn from_env() -> Option<Self> {
        ClusterConfig::from_env().map(Self::new)
    }

    pub fn config(&self) -> &ClusterConfig {
        &self.config
    }

    pub fn health_all(&self) -> Result<Vec<(String, bool)>, String> {
        let mut out = Vec::new();
        for w in &self.config.workers {
            let ok = matches!(
                rpc::call(&w.addr, &Request::Health),
                Ok(Response::Ok { .. })
            );
            out.push((w.id.clone(), ok));
        }
        Ok(out)
    }

    /// Fan out segment BM25 search across workers and merge locally.
    pub fn segment_bm25_search(
        &self,
        table: &str,
        query: &str,
        k: usize,
        segments: &[u32],
    ) -> Result<CandidateSet, String> {
        Coordinator::new(&self.config).search_segments(table, query, k, segments)
    }
}
