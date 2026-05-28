use toradb_core::{Catalog, CompressionConfig, IndexMode, Schema, TableManifest};
use crate::ast::*;

pub struct Binder {
    pub catalog: Catalog,
}

impl Binder {
    pub fn new() -> Self {
        Self { catalog: Catalog::new() }
    }

    pub fn bind(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        for stmt in stmts {
            match stmt {
                Stmt::CreateTable(t) => {
                    let mode = match t.mode.to_uppercase().as_str() {
                        "TEXT" => IndexMode::Text,
                        "VECTOR" => IndexMode::Vector,
                        "HYBRID" => IndexMode::Hybrid,
                        other => {
                            return Err(format!("unsupported CREATE TABLE USING {other}"));
                        }
                    };
                    let table_name = if let Some(ns) = &t.namespace {
                        format!("{ns}.{}", t.name)
                    } else {
                        t.name.clone()
                    };
                    self.catalog.register(TableManifest {
                        name: table_name,
                        schema: Schema::default(),
                        index_mode: mode,
                        vector_dim: None,
                        sparse_enabled: true,
                        graph_enabled: t.columns.iter().any(|(_, ty)| ty.to_uppercase().contains("GRAPH")),
                        compression: CompressionConfig::default(),
                        segments: vec![],
                    });
                }
                Stmt::CreateIndex(idx) => {
                    let table_key = idx.table.to_uppercase();
                    let Some(manifest) = self.catalog.get_mut(&table_key) else {
                        continue;
                    };
                    match idx.using.to_uppercase().as_str() {
                        "BM25" | "SPARSE" | "TEXT" | "SPLADE" | "SEISMIC" => {
                            manifest.sparse_enabled = true;
                            manifest.index_mode = IndexMode::Text;
                        }
                        "HNSW" | "VECTOR" | "DENSE" | "ANN" => {
                            manifest.index_mode = IndexMode::Vector;
                        }
                        "IVF" => {
                            manifest.index_mode = IndexMode::Vector;
                        }
                        "DISKANN" => {
                            manifest.graph_enabled = true;
                            manifest.index_mode = IndexMode::Vector;
                        }
                        "HYBRID" => {
                            manifest.sparse_enabled = true;
                            manifest.index_mode = IndexMode::Hybrid;
                        }
                        other => return Err(format!("unsupported CREATE INDEX USING {other}")),
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}
