mod manifest;
mod metadata_codec;
mod reader;
mod schema;
mod typed_schema;
mod writer;

pub use manifest::{
    IndexMode, QueryMode, SegmentIdRange, SegmentMeta, TableManifestFile,
    ROUTED_QUERY_MIN_SEGMENTS, TIER_BYTE_BOUNDS,
};
pub use reader::{
    bm25_snapshot_from_segment, decode_segment_bytes, iter_segment_batches, parquet_row_count,
    read_segment, read_segment_id_bounds, read_segment_io_uring, read_segment_matching_ids,
    read_segment_texts, scan_segment_id_metadata,
};
pub use schema::doc_schema;
pub use typed_schema::table_doc_schema;
pub use writer::{write_segment, write_segment_from_batches, write_segment_with_compression, ColumnarDoc};
