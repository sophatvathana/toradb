use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use arrow::array::UInt64Array;
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use crate::columnar::{
    iter_segment_batches, write_segment_from_batches, SegmentMeta, TableManifestFile,
    TIER_BYTE_BOUNDS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactMode {
    /// Merge only groups that match size/count policy.
    Auto,
    /// Merge all eligible small-segment groups.
    Normal,
    /// Merge every segment into one (when count > 1).
    Full,
}

#[derive(Debug, Clone)]
pub struct TierPolicy {
    /// Byte-size upper bounds for tiers 0–3 (index = tier number).
    pub tier_bounds: [u64; 4],
    /// Segments at the same tier must reach this count before a merge is triggered.
    pub tier_merge_threshold: usize,
}

impl Default for TierPolicy {
    fn default() -> Self {
        Self {
            tier_bounds: [
                4 * 1024 * 1024,
                16 * 1024 * 1024,
                64 * 1024 * 1024,
                u64::MAX,
            ],
            tier_merge_threshold: 4,
        }
    }
}

impl TierPolicy {
    pub fn tier_for_bytes(&self, bytes: u64) -> u8 {
        for (i, &bound) in self.tier_bounds.iter().enumerate() {
            if bytes < bound {
                return i as u8;
            }
        }
        3
    }

    pub fn output_tier(&self, input_tier: u8) -> u8 {
        (input_tier + 1).min(3)
    }
}

#[derive(Debug, Clone)]
pub struct CompactPolicy {
    pub max_segments: usize,
    pub min_segments_to_merge: usize,
    pub small_segment_bytes: u64,
    pub tier: TierPolicy,
    pub merge_batch_size: usize,
}

impl Default for CompactPolicy {
    fn default() -> Self {
        Self {
            max_segments: 8,
            min_segments_to_merge: 3,
            small_segment_bytes: 4 * 1024 * 1024,
            tier: TierPolicy::default(),
            merge_batch_size: 65536,
        }
    }
}

impl CompactPolicy {
    pub fn from_env() -> Self {
        let mut p = Self::default();
        if let Ok(v) = std::env::var("TORADB_COMPACT_MAX_SEGMENTS") {
            if let Ok(n) = v.parse() { p.max_segments = n; }
        }
        if let Ok(v) = std::env::var("TORADB_COMPACT_MIN_MERGE") {
            if let Ok(n) = v.parse() { p.min_segments_to_merge = n; }
        }
        if let Ok(v) = std::env::var("TORADB_COMPACT_SMALL_BYTES") {
            if let Ok(n) = v.parse() { p.small_segment_bytes = n; }
        }
        if let Ok(v) = std::env::var("TORADB_COMPACT_TIER_THRESHOLD") {
            if let Ok(n) = v.parse() { p.tier.tier_merge_threshold = n; }
        }
        if let Ok(v) = std::env::var("TORADB_COMPACT_TIER0_BYTES") {
            if let Ok(n) = v.parse() { p.tier.tier_bounds[0] = n; }
        }
        if let Ok(v) = std::env::var("TORADB_COMPACT_TIER1_BYTES") {
            if let Ok(n) = v.parse() { p.tier.tier_bounds[1] = n; }
        }
        if let Ok(v) = std::env::var("TORADB_COMPACT_TIER2_BYTES") {
            if let Ok(n) = v.parse() { p.tier.tier_bounds[2] = n; }
        }
        if let Ok(v) = std::env::var("TORADB_COMPACT_BATCH_SIZE") {
            if let Ok(n) = v.parse() { p.merge_batch_size = n; }
        }
        p
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompactReport {
    pub merges: usize,
    pub segments_before: usize,
    pub segments_after: usize,
    pub removed: Vec<String>,
    pub added: Vec<String>,
    pub tier_transitions: Vec<(String, u8)>,
}

#[derive(Debug, Clone)]
pub struct MergeGroup {
    pub segments: Vec<String>,
    pub input_tier: u8,
    pub output_tier: u8,
}

#[derive(Debug, Clone)]
pub struct MergeCandidate {
    pub group: MergeGroup,
    pub score: f64,
}

const MAX_FINITE_TIER_BYTES: u64 = 512 * 1024 * 1024;

fn size_score(meta: &SegmentMeta, policy: &TierPolicy) -> f64 {
    let tier = meta.tier as usize;
    let lower = if tier == 0 { 0u64 } else { policy.tier_bounds[tier - 1] };
    let upper = policy.tier_bounds[tier].min(MAX_FINITE_TIER_BYTES);
    let ideal_mid = (lower as f64 + upper as f64) / 2.0;
    ideal_mid / (meta.byte_size as f64).max(1.0)
}

fn delete_score(meta: &SegmentMeta) -> f64 {
    if meta.row_count == 0 {
        return 0.0;
    }
    meta.deleted_count as f64 / meta.row_count as f64
}

fn tier_weight(tier: u8) -> f64 {
    // T0 = 4.0, T1 = 3.0, T2 = 2.0, T3 = 1.0
    (4 - tier.min(3)) as f64
}

fn group_score(metas: &[&SegmentMeta], policy: &TierPolicy) -> f64 {
    if metas.is_empty() {
        return 0.0;
    }
    let mean_size: f64 = metas.iter().map(|m| size_score(m, policy)).sum::<f64>()
        / metas.len() as f64;
    let mean_delete: f64 = metas.iter().map(|m| delete_score(m)).sum::<f64>()
        / metas.len() as f64;
    let tw = tier_weight(metas[0].tier);
    mean_size + mean_delete + tw
}

pub fn should_compact(
    manifest: &TableManifestFile,
    seg_dir: &Path,
    policy: &CompactPolicy,
) -> bool {
    if manifest.segments.len() < 2 {
        return false;
    }
    if manifest.segments.len() >= policy.max_segments {
        return true;
    }
    let small: Vec<_> = manifest
        .segments
        .iter()
        .filter(|s| segment_size(seg_dir, s) < policy.small_segment_bytes)
        .collect();
    small.len() >= policy.min_segments_to_merge
}

pub fn should_compact_tiered(
    manifest: &TableManifestFile,
    seg_dir: &Path,
    policy: &CompactPolicy,
) -> bool {
    if manifest.segment_meta.is_empty() {
        return should_compact(manifest, seg_dir, policy);
    }
    if manifest.segment_meta.len() < 2 {
        return false;
    }
    for tier in 0u8..=3u8 {
        let count = manifest.segment_meta.iter().filter(|m| m.tier == tier).count();
        if count >= policy.tier.tier_merge_threshold {
            return true;
        }
    }
    manifest.segment_meta.len() >= policy.max_segments
}

fn segment_size(seg_dir: &Path, name: &str) -> u64 {
    seg_dir.join(name).metadata().map(|m| m.len()).unwrap_or(0)
}

fn next_segment_name(manifest: &TableManifestFile) -> String {
    let max_num = manifest
        .segments
        .iter()
        .filter_map(|s| {
            s.strip_prefix("seg_")
                .and_then(|rest| rest.strip_suffix(".parquet"))
                .and_then(|n| n.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);
    format!("seg_{:05}.parquet", max_num + 1)
}

/// Core policy engine: produce scored merge candidates from the manifest.
pub fn pick_merge_candidates(
    manifest: &TableManifestFile,
    seg_dir: &Path,
    policy: &CompactPolicy,
    mode: CompactMode,
) -> Vec<MergeCandidate> {
    if manifest.segments.len() < 2 {
        return Vec::new();
    }

    match mode {
        CompactMode::Full => {
            if manifest.segments.len() > 1 {
                vec![MergeCandidate {
                    group: MergeGroup {
                        segments: manifest.segments.clone(),
                        input_tier: 0,
                        output_tier: 3,
                    },
                    score: f64::MAX,
                }]
            } else {
                Vec::new()
            }
        }

        CompactMode::Normal | CompactMode::Auto => {
            if !manifest.segment_meta.is_empty() {
                let mut candidates: Vec<MergeCandidate> = Vec::new();

                for tier in 0u8..=3u8 {
                    let mut at_tier: Vec<&SegmentMeta> = manifest
                        .segment_meta
                        .iter()
                        .filter(|m| m.tier == tier)
                        .collect();
                    at_tier.sort_by_key(|m| m.created_at);

                    let threshold: usize = policy.tier.tier_merge_threshold;
                    for chunk in at_tier.chunks(threshold) {
                        if chunk.len() == threshold {
                            let score = group_score(chunk, &policy.tier);
                            candidates.push(MergeCandidate {
                                group: MergeGroup {
                                    segments: chunk.iter().map(|m| m.file.clone()).collect(),
                                    input_tier: tier,
                                    output_tier: policy.tier.output_tier(tier),
                                },
                                score,
                            });
                        }
                    }
                }

                candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

                if mode == CompactMode::Auto
                    && candidates.is_empty()
                    && manifest.segment_meta.len() >= policy.max_segments
                {
                    let mut all: Vec<&SegmentMeta> = manifest.segment_meta.iter().collect();
                    all.sort_by_key(|m| m.created_at);
                    let n = policy.min_segments_to_merge.max(2);
                    let oldest: Vec<String> = all.iter().take(n).map(|m| m.file.clone()).collect();
                    if oldest.len() >= 2 {
                        let max_tier = oldest
                            .iter()
                            .filter_map(|f| manifest.segment_meta.iter().find(|m| m.file == *f))
                            .map(|m| m.tier)
                            .max()
                            .unwrap_or(0);
                        candidates.push(MergeCandidate {
                            group: MergeGroup {
                                segments: oldest,
                                input_tier: max_tier,
                                output_tier: policy.tier.output_tier(max_tier),
                            },
                            score: 0.1, // low-priority fallback
                        });
                    }
                }

                return candidates;
            }

            let mut candidates: Vec<MergeCandidate> = Vec::new();
            let mut run: Vec<String> = Vec::new();
            for seg in &manifest.segments {
                let small = segment_size(seg_dir, seg) < policy.small_segment_bytes;
                if small {
                    run.push(seg.clone());
                } else if run.len() >= policy.min_segments_to_merge {
                    candidates.push(MergeCandidate {
                        group: MergeGroup {
                            segments: std::mem::take(&mut run),
                            input_tier: 0,
                            output_tier: 1,
                        },
                        score: 1.0,
                    });
                } else {
                    run.clear();
                }
            }
            if run.len() >= policy.min_segments_to_merge {
                candidates.push(MergeCandidate {
                    group: MergeGroup {
                        segments: run,
                        input_tier: 0,
                        output_tier: 1,
                    },
                    score: 1.0,
                });
            }
            if mode == CompactMode::Auto
                && candidates.is_empty()
                && manifest.segments.len() >= policy.max_segments
            {
                let chunk = policy.min_segments_to_merge.max(2);
                for window in manifest.segments.chunks(chunk) {
                    if window.len() >= 2 {
                        candidates.push(MergeCandidate {
                            group: MergeGroup {
                                segments: window.to_vec(),
                                input_tier: 0,
                                output_tier: 1,
                            },
                            score: 0.5,
                        });
                    }
                }
            }
            candidates
        }
    }
}

pub fn pick_tier_merge_groups(
    manifest: &TableManifestFile,
    seg_dir: &Path,
    policy: &CompactPolicy,
    mode: CompactMode,
) -> Vec<MergeGroup> {
    pick_merge_candidates(manifest, seg_dir, policy, mode)
        .into_iter()
        .map(|c| c.group)
        .collect()
}

pub fn pick_merge_groups(
    manifest: &TableManifestFile,
    seg_dir: &Path,
    policy: &CompactPolicy,
    mode: CompactMode,
) -> Vec<Vec<String>> {
    pick_tier_merge_groups(manifest, seg_dir, policy, mode)
        .into_iter()
        .map(|g| g.segments)
        .collect()
}

struct MergeSource {
    path: PathBuf,
    peeked: Option<RecordBatch>,
    reader: Box<dyn Iterator<Item = Result<RecordBatch, String>>>,
    min_id: u64,
}

impl MergeSource {
    fn open(path: PathBuf, batch_size: usize) -> Result<Self, String> {
        let mut reader: Box<dyn Iterator<Item = Result<RecordBatch, String>>> =
            Box::new(iter_segment_batches(&path, batch_size)?);
        let peeked = reader.next().transpose()?;
        let min_id = peeked
            .as_ref()
            .and_then(|b| batch_min_id(b))
            .unwrap_or(u64::MAX);
        Ok(Self { path, peeked, reader, min_id })
    }

    /// Take the peeked batch and advance.
    fn take_batch(&mut self) -> Option<RecordBatch> {
        let batch = self.peeked.take();
        let next = self.reader.next().and_then(|r| r.ok());
        self.peeked = next;
        self.min_id = self
            .peeked
            .as_ref()
            .and_then(|b| batch_min_id(b))
            .unwrap_or(u64::MAX);
        batch
    }
}

fn batch_min_id(batch: &RecordBatch) -> Option<u64> {
    batch
        .column_by_name("id")?
        .as_any()
        .downcast_ref::<UInt64Array>()?
        .values()
        .iter()
        .copied()
        .next()
}

fn batch_max_id(batch: &RecordBatch) -> Option<u64> {
    batch
        .column_by_name("id")?
        .as_any()
        .downcast_ref::<UInt64Array>()?
        .values()
        .iter()
        .copied()
        .last()
}

fn merge_segments_streaming(
    seg_paths: &[PathBuf],
    out_path: &Path,
    compression: Option<&toradb_core::CompressionConfig>,
    batch_size: usize,
) -> Result<(u64, u64, u64), String> {
    if seg_paths.is_empty() {
        return Err("merge_segments_streaming: no source segments".into());
    }

    let mut sources: Vec<MergeSource> = seg_paths
        .iter()
        .map(|p| MergeSource::open(p.clone(), batch_size))
        .collect::<Result<_, _>>()?;

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    use crate::columnar::doc_schema;
    let schema = doc_schema();
    let file = std::fs::File::create(out_path).map_err(|e| e.to_string())?;
    let mut props_builder = WriterProperties::builder();
    if let Some(cfg) = compression {
        if cfg.enabled {
            props_builder =
                props_builder.set_compression(Compression::ZSTD(Default::default()));
        }
    }
    let props = props_builder.build();
    let mut writer =
        ArrowWriter::try_new(file, schema, Some(props)).map_err(|e| e.to_string())?;

    let mut global_min = u64::MAX;
    let mut global_max = 0u64;
    let mut row_count = 0u64;

    loop {
        let best = sources
            .iter()
            .enumerate()
            .filter(|(_, s)| s.peeked.is_some())
            .min_by_key(|(_, s)| s.min_id);

        let (idx, _) = match best {
            Some(x) => x,
            None => break, // all sources exhausted
        };
        let next_min = sources
            .iter()
            .enumerate()
            .filter(|(i, s)| *i != idx && s.peeked.is_some())
            .map(|(_, s)| s.min_id)
            .min()
            .unwrap_or(u64::MAX);

        loop {
            let src = &mut sources[idx];
            match &src.peeked {
                None => break,
                Some(b) => {
                    if batch_min_id(b).unwrap_or(u64::MAX) >= next_min {
                        break;
                    }
                }
            }
            if let Some(batch) = sources[idx].take_batch() {
                if let Some(bmin) = batch_min_id(&batch) {
                    if bmin < global_min { global_min = bmin; }
                }
                if let Some(bmax) = batch_max_id(&batch) {
                    if bmax > global_max { global_max = bmax; }
                }
                row_count += batch.num_rows() as u64;
                writer.write(&batch).map_err(|e| e.to_string())?;
            }
        }
    }

    writer.close().map_err(|e| e.to_string())?;

    let min_id = if global_min == u64::MAX { 0 } else { global_min };
    Ok((min_id, global_max, row_count))
}

/// Merge Parquet segments, update manifest, delete superseded files. Does not rebuild index sidecars.
pub fn compact_table_segments(
    base: &Path,
    table: &str,
    policy: &CompactPolicy,
    mode: CompactMode,
) -> Result<CompactReport, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(CompactReport::default());
    }
    let mut manifest = TableManifestFile::load(&manifest_path)?;
    let seg_dir = TableManifestFile::segments_dir(base, table);
    if manifest.segments.len() < 2 {
        return Ok(CompactReport {
            segments_before: manifest.segments.len(),
            segments_after: manifest.segments.len(),
            ..Default::default()
        });
    }
    if mode == CompactMode::Auto && !should_compact_tiered(&manifest, &seg_dir, policy) {
        return Ok(CompactReport {
            segments_before: manifest.segments.len(),
            segments_after: manifest.segments.len(),
            ..Default::default()
        });
    }
    let candidates = pick_merge_candidates(&manifest, &seg_dir, policy, mode);
    if candidates.is_empty() {
        return Ok(CompactReport {
            segments_before: manifest.segments.len(),
            segments_after: manifest.segments.len(),
            ..Default::default()
        });
    }

    let before = manifest.segments.len();
    let mut report = CompactReport {
        segments_before: before,
        ..Default::default()
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for candidate in candidates {
        let group = candidate.group;
        if group.segments.len() < 2 {
            continue;
        }

        // Verify all source files exist before starting the merge.
        for seg in &group.segments {
            if !seg_dir.join(seg).exists() {
                return Err(format!("missing segment file {seg}"));
            }
        }

        let seg_paths: Vec<PathBuf> = group.segments.iter().map(|s| seg_dir.join(s)).collect();
        let new_name = next_segment_name(&manifest);
        let new_path = seg_dir.join(&new_name);

        let (min_id, max_id, row_count) = merge_segments_streaming(
            &seg_paths,
            &new_path,
            manifest.compression.as_ref(),
            policy.merge_batch_size,
        )?;

        let byte_size = new_path.metadata().map(|m| m.len()).unwrap_or(0);
        let generation = manifest.next_generation();

        let actual_tier = policy
            .tier
            .tier_for_bytes(byte_size)
            .max(group.output_tier);

        for old in &group.segments {
            manifest.remove_segment(old);
            let p = seg_dir.join(old);
            if p.exists() {
                std::fs::remove_file(&p).map_err(|e| e.to_string())?;
            }
            report.removed.push(old.clone());
        }

        manifest.push_segment_meta(SegmentMeta {
            file: new_name.clone(),
            min_id,
            max_id,
            tier: actual_tier,
            generation,
            created_at: now,
            byte_size,
            row_count,
            deleted_count: 0,
        });

        report.tier_transitions.push((new_name.clone(), actual_tier));
        report.added.push(new_name);
        report.merges += 1;
    }

    manifest.save(&manifest_path)?;
    report.segments_after = manifest.segments.len();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::columnar::{write_segment_with_compression, ColumnarDoc};

    fn make_segment_meta(file: &str, tier: u8, created_at: u64, byte_size: u64) -> SegmentMeta {
        SegmentMeta {
            file: file.to_string(),
            min_id: 0,
            max_id: 0,
            tier,
            generation: 0,
            created_at,
            byte_size,
            row_count: 0,
            deleted_count: 0,
        }
    }

    #[test]
    fn pick_small_segment_group() {
        let dir = tempfile::tempdir().unwrap();
        let seg_dir = dir.path().join("segments");
        std::fs::create_dir_all(&seg_dir).unwrap();
        let names = ["seg_00001.parquet", "seg_00002.parquet", "seg_00003.parquet"];
        for n in names {
            write_segment_with_compression(
                &seg_dir.join(n),
                &[ColumnarDoc {
                    id: 1,
                    text: "x".into(),
                    metadata: Default::default(),
                    embedding: None,
                }],
                None,
            )
            .unwrap();
        }
        let manifest = TableManifestFile {
            segments: names.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        };
        let policy = CompactPolicy {
            small_segment_bytes: u64::MAX,
            min_segments_to_merge: 3,
            ..Default::default()
        };
        let groups = pick_merge_groups(&manifest, &seg_dir, &policy, CompactMode::Normal);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 3);
    }

    #[test]
    fn tier_classification_by_size() {
        let policy = TierPolicy::default();
        assert_eq!(policy.tier_for_bytes(0), 0);
        assert_eq!(policy.tier_for_bytes(4 * 1024 * 1024 - 1), 0);
        assert_eq!(policy.tier_for_bytes(4 * 1024 * 1024), 1);
        assert_eq!(policy.tier_for_bytes(16 * 1024 * 1024 - 1), 1);
        assert_eq!(policy.tier_for_bytes(16 * 1024 * 1024), 2);
        assert_eq!(policy.tier_for_bytes(64 * 1024 * 1024 - 1), 2);
        assert_eq!(policy.tier_for_bytes(64 * 1024 * 1024), 3);
        assert_eq!(policy.tier_for_bytes(u64::MAX), 3);
    }

    #[test]
    fn pick_tier_groups_respects_threshold() {
        let mut manifest = TableManifestFile::default();
        for i in 1u32..=5 {
            let name = format!("seg_{:05}.parquet", i);
            manifest.segment_meta.push(make_segment_meta(&name, 0, i as u64, 1024));
            manifest.segments.push(name);
        }
        for i in 6u32..=7 {
            let name = format!("seg_{:05}.parquet", i);
            manifest.segment_meta.push(make_segment_meta(&name, 1, i as u64, 5 * 1024 * 1024));
            manifest.segments.push(name);
        }
        let policy = CompactPolicy {
            tier: TierPolicy { tier_merge_threshold: 4, ..Default::default() },
            ..Default::default()
        };
        let groups =
            pick_tier_merge_groups(&manifest, Path::new("/tmp"), &policy, CompactMode::Normal);
        assert_eq!(groups.len(), 1, "only one group from T0");
        assert_eq!(groups[0].segments.len(), 4);
        assert_eq!(groups[0].input_tier, 0);
        assert_eq!(groups[0].output_tier, 1);
    }

    #[test]
    fn pick_tier_groups_oldest_first() {
        let mut manifest = TableManifestFile::default();
        let times = [10u64, 5, 20, 1, 15, 8];
        for (i, &t) in times.iter().enumerate() {
            let name = format!("seg_{:05}.parquet", i + 1);
            manifest.segment_meta.push(make_segment_meta(&name, 0, t, 1024));
            manifest.segments.push(name);
        }
        let policy = CompactPolicy {
            tier: TierPolicy { tier_merge_threshold: 4, ..Default::default() },
            ..Default::default()
        };
        let groups =
            pick_tier_merge_groups(&manifest, Path::new("/tmp"), &policy, CompactMode::Normal);
        assert_eq!(groups.len(), 1);
        let mut sorted_meta: Vec<&SegmentMeta> = manifest.segment_meta.iter().collect();
        sorted_meta.sort_by_key(|m| m.created_at);
        let expected: Vec<String> = sorted_meta.iter().take(4).map(|m| m.file.clone()).collect();
        assert_eq!(groups[0].segments, expected);
    }

    #[test]
    fn full_compact_produces_tier3() {
        let mut manifest = TableManifestFile::default();
        for i in 1u32..=3 {
            let name = format!("seg_{:05}.parquet", i);
            manifest.segment_meta.push(make_segment_meta(&name, 0, i as u64, 1024));
            manifest.segments.push(name);
        }
        let policy = CompactPolicy::default();
        let groups =
            pick_tier_merge_groups(&manifest, Path::new("/tmp"), &policy, CompactMode::Full);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].output_tier, 3);
        assert_eq!(groups[0].segments.len(), 3);
    }

    #[test]
    fn no_cross_tier_merges() {
        let mut manifest = TableManifestFile::default();
        for i in 1u32..=3 {
            let name = format!("seg_{:05}.parquet", i);
            manifest.segment_meta.push(make_segment_meta(&name, 0, i as u64, 1024));
            manifest.segments.push(name);
        }
        for i in 4u32..=6 {
            let name = format!("seg_{:05}.parquet", i);
            manifest.segment_meta.push(make_segment_meta(&name, 1, i as u64, 5 * 1024 * 1024));
            manifest.segments.push(name);
        }
        let policy = CompactPolicy {
            tier: TierPolicy { tier_merge_threshold: 4, ..Default::default() },
            ..Default::default()
        };
        let groups =
            pick_tier_merge_groups(&manifest, Path::new("/tmp"), &policy, CompactMode::Normal);
        assert_eq!(groups.len(), 0, "no cross-tier merges");
    }

    #[test]
    fn candidates_sorted_by_score() {
        // T0 segments (undersized) should score higher than T3 segments (large).
        let mut manifest = TableManifestFile::default();
        // 4 T3 segments (large) — low size_score
        for i in 1u32..=4 {
            let name = format!("seg_{:05}.parquet", i);
            manifest.segment_meta.push(make_segment_meta(&name, 3, i as u64, 200 * 1024 * 1024));
            manifest.segments.push(name);
        }
        // 4 T0 segments (tiny) — high size_score
        for i in 5u32..=8 {
            let name = format!("seg_{:05}.parquet", i);
            manifest.segment_meta.push(make_segment_meta(&name, 0, i as u64, 512));
            manifest.segments.push(name);
        }
        let policy = CompactPolicy {
            tier: TierPolicy { tier_merge_threshold: 4, ..Default::default() },
            ..Default::default()
        };
        let cands =
            pick_merge_candidates(&manifest, Path::new("/tmp"), &policy, CompactMode::Normal);
        assert_eq!(cands.len(), 2);
        // T0 group should have higher score (comes first after sort).
        assert_eq!(cands[0].group.input_tier, 0);
        assert!(cands[0].score > cands[1].score);
    }

    #[test]
    fn streaming_merge_produces_correct_output() {
        let dir = tempfile::tempdir().unwrap();
        let seg_dir = dir.path().join("segments");
        std::fs::create_dir_all(&seg_dir).unwrap();

        // Write 3 small segments with sequential IDs.
        let names = ["seg_00001.parquet", "seg_00002.parquet", "seg_00003.parquet"];
        let mut id = 0u64;
        for name in &names {
            let docs: Vec<ColumnarDoc> = (0..5).map(|_| {
                let d = ColumnarDoc {
                    id,
                    text: format!("doc {id}"),
                    metadata: Default::default(),
                    embedding: None,
                };
                id += 1;
                d
            }).collect();
            write_segment_with_compression(&seg_dir.join(name), &docs, None).unwrap();
        }

        let paths: Vec<PathBuf> = names.iter().map(|n| seg_dir.join(n)).collect();
        let out = seg_dir.join("merged.parquet");
        let (min_id, max_id, row_count) =
            merge_segments_streaming(&paths, &out, None, 4096).unwrap();

        assert_eq!(min_id, 0);
        assert_eq!(max_id, 14);
        assert_eq!(row_count, 15);
        assert!(out.exists());

        // Verify the output can be read back with correct doc count.
        let docs = crate::columnar::read_segment(&out).unwrap();
        assert_eq!(docs.len(), 15);
        for (i, doc) in docs.iter().enumerate() {
            assert_eq!(doc.id, i as u64);
        }
    }
}
