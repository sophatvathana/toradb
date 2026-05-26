use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use memmap2::Mmap;
use toradb_core::CandidateSet;
use toradb_index::sparse::bm25_tbm3::{Bm25Tbm3View, TBM3_MAGIC};

use crate::columnar::ColumnarDoc;
use crate::numa::{NumaConfig, prefetch_mmap_sequential};

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub segment_entries: usize,
    pub index_bytes: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            segment_entries: 32,
            index_bytes: 256 * 1024 * 1024,
        }
    }
}

impl CacheConfig {
    pub fn from_env() -> Self {
        let mut c = Self::default();
        if let Ok(v) = std::env::var("TORADB_CACHE_SEGMENT_ENTRIES") {
            if let Ok(n) = v.parse::<usize>() {
                c.segment_entries = n.max(1);
            }
        }
        if let Ok(v) = std::env::var("TORADB_CACHE_INDEX_BYTES") {
            if let Ok(n) = v.parse::<usize>() {
                c.index_bytes = n.max(1024);
            }
        }
        c
    }
}

#[derive(Debug, Default, Clone)]
pub struct CacheHierarchy {
    pub hits: u64,
    pub misses: u64,
}

#[derive(Debug)]
pub struct SegmentCache {
    capacity: usize,
    order: VecDeque<PathBuf>,
    map: HashMap<PathBuf, Arc<Vec<ColumnarDoc>>>,
    pub stats: CacheHierarchy,
}

impl SegmentCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            map: HashMap::new(),
            stats: CacheHierarchy::default(),
        }
    }

    pub fn get(&mut self, path: &Path) -> Option<Arc<Vec<ColumnarDoc>>> {
        let key = path.to_path_buf();
        if self.map.contains_key(&key) {
            self.stats.hits += 1;
            self.touch(&key);
            return self.map.get(&key).map(Arc::clone);
        }
        self.stats.misses += 1;
        None
    }

    pub fn insert(&mut self, path: PathBuf, docs: Vec<ColumnarDoc>) {
        let arc = Arc::new(docs);
        if self.map.contains_key(&path) {
            self.map.insert(path.clone(), Arc::clone(&arc));
            self.touch(&path);
            return;
        }
        while self.map.len() >= self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            } else {
                break;
            }
        }
        self.order.push_back(path.clone());
        self.map.insert(path, arc);
    }

    pub fn invalidate(&mut self, path: &Path) {
        let key = path.to_path_buf();
        if self.map.remove(&key).is_some() {
            self.order.retain(|p| p != &key);
        }
    }

    fn touch(&mut self, key: &PathBuf) {
        self.order.retain(|p| p != key);
        self.order.push_back(key.clone());
    }
}

#[derive(Debug)]
pub struct IndexBlobCache {
    byte_budget: usize,
    bytes_used: usize,
    order: VecDeque<PathBuf>,
    map: HashMap<PathBuf, Arc<Mmap>>,
    pub stats: CacheHierarchy,
}

impl IndexBlobCache {
    pub fn new(byte_budget: usize) -> Self {
        Self {
            byte_budget: byte_budget.max(1024),
            bytes_used: 0,
            order: VecDeque::new(),
            map: HashMap::new(),
            stats: CacheHierarchy::default(),
        }
    }

    pub fn get(&mut self, path: &Path) -> Option<Arc<Mmap>> {
        let key = path.to_path_buf();
        if self.map.contains_key(&key) {
            self.stats.hits += 1;
            self.touch(&key);
            return self.map.get(&key).map(Arc::clone);
        }
        self.stats.misses += 1;
        None
    }

    pub fn insert(&mut self, path: PathBuf, mmap: Arc<Mmap>) {
        let size = mmap.len();
        if self.map.contains_key(&path) {
            if let Some(old) = self.map.remove(&path) {
                self.bytes_used = self.bytes_used.saturating_sub(old.len());
            }
            self.order.retain(|p| p != &path);
        }
        while self.bytes_used + size > self.byte_budget && !self.order.is_empty() {
            if let Some(old_key) = self.order.pop_front() {
                if let Some(old) = self.map.remove(&old_key) {
                    self.bytes_used = self.bytes_used.saturating_sub(old.len());
                }
            }
        }
        self.bytes_used += size;
        self.order.push_back(path.clone());
        self.map.insert(path, mmap);
    }

    pub fn invalidate(&mut self, path: &Path) {
        let key = path.to_path_buf();
        if let Some(old) = self.map.remove(&key) {
            self.bytes_used = self.bytes_used.saturating_sub(old.len());
            self.order.retain(|p| p != &key);
        }
    }

    fn touch(&mut self, key: &PathBuf) {
        self.order.retain(|p| p != key);
        self.order.push_back(key.clone());
    }
}

/// Cached per-segment BM25 (`TBM3` mmap).
#[derive(Debug)]
pub struct CachedBm25Segment(Arc<Mmap>);

impl CachedBm25Segment {
    pub fn from_mmap(mmap: Arc<Mmap>) -> Result<Self, String> {
        if mmap.len() < 4 || &mmap[..4] != TBM3_MAGIC {
            return Err("invalid BM25 sidecar (expected TBM3)".into());
        }
        Ok(Self(mmap))
    }

    pub fn search(&self, query: &str, k: usize) -> Result<CandidateSet, String> {
        let view = Bm25Tbm3View::from_mmap(&self.0)?;
        Ok(view.search(query, k))
    }

    fn entry_bytes(path: &Path, _entry: &Self) -> usize {
        std::fs::metadata(path)
            .map(|m| m.len() as usize)
            .unwrap_or(64 * 1024 * 1024)
    }
}

/// LRU of per-segment BM25 indexes (thread-safe for parallel segment scans).
#[derive(Debug)]
pub struct SegmentBm25Cache {
    byte_budget: usize,
    bytes_used: usize,
    order: VecDeque<PathBuf>,
    map: HashMap<PathBuf, Arc<CachedBm25Segment>>,
    pub stats: CacheHierarchy,
}

impl SegmentBm25Cache {
    pub fn new(byte_budget: usize) -> Self {
        Self {
            byte_budget: byte_budget.max(1024),
            bytes_used: 0,
            order: VecDeque::new(),
            map: HashMap::new(),
            stats: CacheHierarchy::default(),
        }
    }

    pub fn get(&self, path: &Path) -> Option<Arc<CachedBm25Segment>> {
        self.map.get(&path.to_path_buf()).cloned()
    }

    pub fn get_or_insert<F>(&mut self, path: &Path, load: F) -> Result<Arc<CachedBm25Segment>, String>
    where
        F: FnOnce() -> Result<CachedBm25Segment, String>,
    {
        let key = path.to_path_buf();
        if let Some(hit) = self.map.get(&key).cloned() {
            self.stats.hits += 1;
            self.touch(&key);
            return Ok(hit);
        }
        self.stats.misses += 1;
        let entry = load()?;
        let size = CachedBm25Segment::entry_bytes(path, &entry);
        if self.map.contains_key(&key) {
            if let Some(old) = self.map.remove(&key) {
                self.bytes_used = self
                    .bytes_used
                    .saturating_sub(CachedBm25Segment::entry_bytes(&key, &old));
            }
            self.order.retain(|p| p != &key);
        }
        while self.bytes_used + size > self.byte_budget && !self.order.is_empty() {
            if let Some(old_key) = self.order.pop_front() {
                if let Some(old) = self.map.remove(&old_key) {
                    self.bytes_used = self.bytes_used.saturating_sub(
                        CachedBm25Segment::entry_bytes(&old_key, &old),
                    );
                }
            }
        }
        self.bytes_used += size;
        self.order.push_back(key.clone());
        let arc = Arc::new(entry);
        self.map.insert(key, Arc::clone(&arc));
        Ok(arc)
    }

    fn touch(&mut self, key: &PathBuf) {
        self.order.retain(|p| p != key);
        self.order.push_back(key.clone());
    }
}

#[derive(Debug)]
pub struct StorageCaches {
    pub segments: SegmentCache,
    pub index_blobs: IndexBlobCache,
    pub segment_bm25: RwLock<SegmentBm25Cache>,
    pub numa: NumaConfig,
}

impl StorageCaches {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            segments: SegmentCache::new(config.segment_entries),
            index_blobs: IndexBlobCache::new(config.index_bytes),
            segment_bm25: RwLock::new(SegmentBm25Cache::new(config.index_bytes)),
            numa: NumaConfig::from_env(),
        }
    }

    pub fn default_from_env() -> Self {
        Self::new(CacheConfig::from_env())
    }

    pub fn combined_stats(&self) -> CacheHierarchy {
        let bm25 = self
            .segment_bm25
            .read()
            .map(|c| c.stats.clone())
            .unwrap_or_default();
        CacheHierarchy {
            hits: self.segments.stats.hits + self.index_blobs.stats.hits + bm25.hits,
            misses: self.segments.stats.misses + self.index_blobs.stats.misses + bm25.misses,
        }
    }

    pub fn invalidate_segment(&mut self, path: &Path) {
        self.segments.invalidate(path);
    }

    pub fn invalidate_index_blob(&mut self, path: &Path) {
        self.index_blobs.invalidate(path);
    }
}

pub fn read_segment_cached(
    path: &Path,
    caches: Option<&mut StorageCaches>,
) -> Result<Vec<ColumnarDoc>, String> {
    if let Some(caches) = caches {
        if let Some(hit) = caches.segments.get(path) {
            return Ok((&**hit).to_vec());
        }
        let docs = crate::columnar::read_segment_io_uring(path)?;
        caches.segments.insert(path.to_path_buf(), docs.clone());
        return Ok(docs);
    }
    crate::columnar::read_segment_io_uring(path)
}

pub fn get_or_mmap(path: &Path, mut caches: Option<&mut StorageCaches>) -> Result<Arc<Mmap>, String> {
    if let Some(caches) = &mut caches {
        if let Some(hit) = caches.index_blobs.get(path) {
            return Ok(hit);
        }
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let arc = Arc::new(unsafe { Mmap::map(&file).map_err(|e| e.to_string())? });
        if caches.numa.prefetch {
            prefetch_mmap_sequential(arc.as_ref());
        }
        caches
            .index_blobs
            .insert(path.to_path_buf(), Arc::clone(&arc));
        return Ok(arc);
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    Ok(Arc::new(
        unsafe { Mmap::map(&file).map_err(|e| e.to_string())? },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_lru_evicts_oldest() {
        let mut cache = SegmentCache::new(2);
        cache.insert(PathBuf::from("/a"), vec![]);
        cache.insert(PathBuf::from("/b"), vec![]);
        cache.insert(PathBuf::from("/c"), vec![]);
        assert!(cache.get(Path::new("/a")).is_none());
        assert!(cache.get(Path::new("/b")).is_some());
        assert!(cache.get(Path::new("/c")).is_some());
    }
}
