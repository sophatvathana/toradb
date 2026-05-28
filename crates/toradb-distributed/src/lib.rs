//! Multi-node coordinator / worker RPC for segment-scoped retrieval.

pub mod client;
pub mod config;
pub mod coordinator;
pub mod protocol;
pub mod rpc;
pub mod worker;

pub use client::ClusterClient;
pub use config::ClusterConfig;
pub use coordinator::Coordinator;
pub use worker::Worker;
