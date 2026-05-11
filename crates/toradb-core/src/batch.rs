use crate::candidate::CandidateSet;

#[derive(Debug, Clone, Default)]
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
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }
}
