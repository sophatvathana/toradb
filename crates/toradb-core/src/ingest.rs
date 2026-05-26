#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IngestOptions {
    /// Skip HNSW / DiskANN / segment HNSW rebuilds after each batch.
    pub defer_dense_rebuild: bool,
    /// Skip table-level index sidecar writes after each flush.
    pub defer_table_indexes: bool,
    /// Skip automatic compaction after each flush.
    pub defer_compaction: bool,
    /// Skip in-memory BM25 updates per doc (segment sidecars built on flush; merged at finish).
    pub defer_bm25: bool,
    /// Skip sequential graph edge inserts between adjacent doc ids.
    pub defer_graph: bool,
    /// Append WAL flush records without fsync until bulk finish (group commit).
    pub defer_wal_fsync: bool,
}

impl IngestOptions {
    /// Deferred index work until `finish_bulk_ingest`.
    pub fn bulk() -> Self {
        Self {
            defer_dense_rebuild: true,
            defer_table_indexes: true,
            defer_compaction: true,
            defer_bm25: true,
            defer_graph: true,
            defer_wal_fsync: true,
        }
    }
}
