use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use toradb_storage::cache::{CachedBm25Segment, StorageCaches};
use toradb_storage::columnar::TableManifestFile;

use crate::protocol::{Request, Response};
use crate::rpc;

pub struct Worker {
    pub db_path: PathBuf,
    caches: Arc<Mutex<StorageCaches>>,
}

impl Worker {
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
            caches: Arc::new(Mutex::new(StorageCaches::default_from_env())),
        }
    }

    fn segment_bm25_path(&self, table: &str, segment_parquet: &str) -> PathBuf {
        let stem = segment_parquet
            .strip_suffix(".parquet")
            .unwrap_or(segment_parquet);
        self.db_path
            .join(table)
            .join("indexes")
            .join(format!("{stem}.bm25.bin"))
    }

    pub fn handle(&self, req: Request) -> Response {
        match req {
            Request::Health => Response::ok_message("ok"),
            Request::SegmentSearch {
                table,
                segment,
                query,
                k,
            } => match self.search_segment(&table, segment, &query, k as usize) {
                Ok(c) => Response::ok_candidates(c),
                Err(e) => Response::err(e),
            },
        }
    }

    pub fn search_segment(
        &self,
        table: &str,
        segment: u32,
        query: &str,
        k: usize,
    ) -> Result<toradb_core::CandidateSet, String> {
        let manifest_path = TableManifestFile::path_for_table(&self.db_path, table);
        if !manifest_path.exists() {
            return Ok(toradb_core::CandidateSet::default());
        }
        let manifest = TableManifestFile::load(&manifest_path)?;
        let seg_file = manifest
            .segments
            .get(segment as usize)
            .ok_or_else(|| format!("segment index {segment} out of range"))?;
        let bin_path = self.segment_bm25_path(table, seg_file);
        if !bin_path.exists() {
            return Ok(toradb_core::CandidateSet::default());
        }
        let caches = self
            .caches
            .lock()
            .map_err(|_| "cache lock poisoned".to_string())?;
        if let Some(entry) = caches.segment_bm25.read().ok().and_then(|g| g.get(&bin_path)) {
            return entry.search(query, k);
        }
        let path = bin_path.clone();
        let mmap = toradb_storage::cache::get_or_mmap(&path, None)?;
        let segment = CachedBm25Segment::from_mmap(mmap)?;
        let result = segment.search(query, k);
        if let Ok(mut guard) = caches.segment_bm25.write() {
            let _ = guard.get_or_insert(&bin_path, || Ok(segment));
        }
        result
    }

    pub fn serve_blocking(self, addr: &str) -> Result<(), String> {
        rpc::serve(addr, move |req| self.handle(req))
    }

    /// Handle a single RPC request then return (for tests).
    pub fn serve_one(self, addr: &str) -> Result<(), String> {
        rpc::serve_one(addr, move |req| self.handle(req))
    }
}
