mod manifest;
mod reader;
mod schema;
mod writer;

pub use manifest::{IndexMode, TableManifestFile};
pub use reader::{
    decode_segment_bytes, parquet_row_count, read_segment, read_segment_id_bounds,
    read_segment_io_uring, read_segment_matching_ids,
    read_segment_texts,
};
pub use schema::doc_schema;
pub use writer::{write_segment, write_segment_with_compression, ColumnarDoc};
