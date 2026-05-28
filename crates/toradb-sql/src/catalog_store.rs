use std::path::{Path, PathBuf};

use toradb_core::Catalog;

pub fn catalog_path(base: &Path) -> PathBuf {
    base.join("catalog.json")
}

pub fn load_catalog(base: &Path) -> Result<Catalog, String> {
    let path = catalog_path(base);
    if !path.exists() {
        return Ok(Catalog::new());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let tables: Vec<toradb_core::TableManifest> =
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let mut catalog = Catalog::new();
    for manifest in tables {
        catalog.register(manifest);
    }
    Ok(catalog)
}

pub fn save_catalog(base: &Path, catalog: &Catalog) -> Result<(), String> {
    let tables: Vec<_> = catalog.iter_tables().cloned().collect();
    let bytes = serde_json::to_vec_pretty(&tables).map_err(|e| e.to_string())?;
    let path = catalog_path(base);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}
