use crate::candidate::CandidateSet;
use crate::provenance::ProvenanceCollector;

#[derive(Debug, Clone)]
pub struct Batch {
    pub candidates: CandidateSet,
    /// Query text propagated through the DAG (SDK and SQL lowering).
    pub query: String,
    pub enable_hyde: bool,
    pub enable_crag: bool,
    pub graph_expand: bool,
    pub graph_depth: u32,
    pub table: String,
    pub query_vector: Option<Vec<f32>>,
    pub tier1_enable_sparse: bool,
    pub tier1_enable_dense: bool,
    /// When true, tier-1 dense retrieval uses the on-disk DiskANN graph sidecar.
    pub tier1_use_diskann: bool,
    /// When true, tier-1 dense uses IVF (falls back to HNSW in-memory when unavailable).
    pub tier1_use_ivf: bool,
    /// When true, run per-segment retrieval across worker threads (single-node distributed).
    pub distributed_segments: bool,
    /// Sparse backend: `bm25` (default), `splade`, or `seismic`.
    pub sparse_backend: String,
    /// RRF fusion constant (default 60).
    pub fusion_k: u32,
    pub provenance: Option<ProvenanceCollector>,
}

impl Default for Batch {
    fn default() -> Self {
        Self {
            candidates: CandidateSet::default(),
            query: String::new(),
            enable_hyde: false,
            enable_crag: false,
            graph_expand: false,
            graph_depth: 0,
            table: String::new(),
            query_vector: None,
            tier1_enable_sparse: true,
            tier1_enable_dense: true,
            tier1_use_diskann: false,
            tier1_use_ivf: false,
            distributed_segments: false,
            sparse_backend: "bm25".into(),
            fusion_k: 60,
            provenance: None,
        }
    }
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn with_provenance(&mut self, f: impl FnOnce(&mut ProvenanceCollector)) {
        if let Some(p) = self.provenance.as_mut() {
            f(p);
        }
    }
}
