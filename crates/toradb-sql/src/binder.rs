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
                    let mode = match t.mode.as_str() {
                        "TEXT" => IndexMode::Text,
                        "VECTOR" => IndexMode::Vector,
                        _ => IndexMode::Hybrid,
                    };
                    self.catalog.register(TableManifest {
                        name: t.name.clone(),
                        schema: Schema::default(),
                        index_mode: mode,
                        vector_dim: None,
                        sparse_enabled: true,
                        graph_enabled: t.columns.iter().any(|(_, ty)| ty.to_uppercase().contains("GRAPH")),
                        compression: CompressionConfig::default(),
                        segments: vec![],
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }
}
