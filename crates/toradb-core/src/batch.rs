use crate::candidate::CandidateSet;

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
    /// When true, run per-segment retrieval across worker threads (single-node distributed).
    pub distributed_segments: bool,
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
            distributed_segments: false,
        }
    }
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }
}
