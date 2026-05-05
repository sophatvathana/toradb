//! Core types: CandidateSet, Batch, ExecCtx, catalog, compression.

pub mod arena;
pub mod batch;
pub mod candidate;
pub mod catalog;
pub mod compression;
pub mod exec_ctx;
pub mod metrics;
pub mod quant;
pub mod schema;

pub use batch::Batch;
pub use candidate::CandidateSet;
pub use catalog::{Catalog, TableManifest};
pub use compression::{CompressionConfig, IndexMode};
pub use exec_ctx::ExecCtx;
pub use schema::{ColumnDef, ColumnKind, DocId, Schema, SegmentId};
pub use metrics::QueryMetrics;
