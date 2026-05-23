#[derive(Debug, Default, Clone)]
pub struct QueryMetrics {
    pub tier1_candidates: u32,
    pub tier2_candidates: u32,
    pub tier3_candidates: u32,
    pub decompressions: u32,
    pub cache_hits: u64,
    pub io_bytes: u64,
    /// Segment shards scanned in the distributed (parallel) tier.
    pub segments_scanned: u32,
    /// Worker threads used for segment-parallel retrieval.
    pub segment_workers: u32,
}
