//! Core types: CandidateSet, Batch, ExecCtx, catalog, compression.

pub mod arena;
pub mod batch;
pub mod candidate;
pub mod catalog;
pub mod compression;
pub mod exec_ctx;
pub mod ingest;
pub mod metrics;
pub mod provenance;
pub mod quant;
pub mod schema;
pub mod typed_compare;

pub use batch::Batch;
pub use candidate::CandidateSet;
pub use catalog::{Catalog, TableManifest};
pub use compression::{CompressionConfig, IndexMode};
pub use exec_ctx::ExecCtx;
pub use ingest::IngestOptions;
pub use schema::{ColumnDef, ColumnKind, ColumnType, DocId, Schema, SegmentId};
pub use typed_compare::typed_cmp;
pub use metrics::QueryMetrics;
pub use provenance::{DropStage, ProvenanceCollector, ProvenanceRecord, TierTrace};
