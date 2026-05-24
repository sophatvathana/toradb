use std::path::Path;

use crate::columnar::{
    read_segment, write_segment_with_compression, ColumnarDoc, TableManifestFile,
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
pub struct CompactPolicy {
    pub max_segments: usize,
    pub min_segments_to_merge: usize,
    pub small_segment_bytes: u64,
}

impl Default for CompactPolicy {
    fn default() -> Self {
        Self {
            max_segments: 8,
            min_segments_to_merge: 3,
            small_segment_bytes: 4 * 1024 * 1024,
        }
    }
}

impl CompactPolicy {
    pub fn from_env() -> Self {
        let mut p = Self::default();
        if let Ok(v) = std::env::var("TORADB_COMPACT_MAX_SEGMENTS") {
            if let Ok(n) = v.parse() {
                p.max_segments = n;
            }
        }
        if let Ok(v) = std::env::var("TORADB_COMPACT_MIN_MERGE") {
            if let Ok(n) = v.parse() {
                p.min_segments_to_merge = n;
            }
        }
        if let Ok(v) = std::env::var("TORADB_COMPACT_SMALL_BYTES") {
            if let Ok(n) = v.parse() {
                p.small_segment_bytes = n;
            }
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

/// Pick contiguous groups of segments to merge (by manifest order).
pub fn pick_merge_groups(
    manifest: &TableManifestFile,
    seg_dir: &Path,
    policy: &CompactPolicy,
    mode: CompactMode,
) -> Vec<Vec<String>> {
    if manifest.segments.len() < 2 {
        return Vec::new();
    }
    match mode {
        CompactMode::Full => {
            if manifest.segments.len() > 1 {
                vec![manifest.segments.clone()]
            } else {
                Vec::new()
            }
        }
        CompactMode::Normal | CompactMode::Auto => {
            let mut groups = Vec::new();
            let mut run: Vec<String> = Vec::new();
            for seg in &manifest.segments {
                let small = segment_size(seg_dir, seg) < policy.small_segment_bytes;
                if small {
                    run.push(seg.clone());
                } else if run.len() >= policy.min_segments_to_merge {
                    groups.push(std::mem::take(&mut run));
                } else {
                    run.clear();
                }
            }
            if run.len() >= policy.min_segments_to_merge {
                groups.push(run);
            }
            if mode == CompactMode::Auto
                && groups.is_empty()
                && manifest.segments.len() >= policy.max_segments
            {
                let chunk = policy.min_segments_to_merge.max(2);
                for window in manifest.segments.chunks(chunk) {
                    if window.len() >= 2 {
                        groups.push(window.to_vec());
                    }
                }
            }
            groups
        }
    }
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
    if mode == CompactMode::Auto && !should_compact(&manifest, &seg_dir, policy) {
        return Ok(CompactReport {
            segments_before: manifest.segments.len(),
            segments_after: manifest.segments.len(),
            ..Default::default()
        });
    }
    let groups = pick_merge_groups(&manifest, &seg_dir, policy, mode);
    if groups.is_empty() {
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

    for group in groups {
        if group.len() < 2 {
            continue;
        }
        let mut docs: Vec<ColumnarDoc> = Vec::new();
        for seg in &group {
            let path = seg_dir.join(seg);
            if !path.exists() {
                return Err(format!("missing segment file {seg}"));
            }
            let mut seg_docs = read_segment(&path)?;
            docs.append(&mut seg_docs);
        }
        docs.sort_by_key(|d| d.id);

        let new_name = next_segment_name(&manifest);
        let new_path = seg_dir.join(&new_name);
        write_segment_with_compression(&new_path, &docs, manifest.compression.as_ref())?;

        manifest.segments.retain(|s| !group.contains(s));
        manifest.segments.push(new_name.clone());

        for old in &group {
            let p = seg_dir.join(old);
            if p.exists() {
                std::fs::remove_file(&p).map_err(|e| e.to_string())?;
            }
            report.removed.push(old.clone());
        }
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
    use crate::columnar::write_segment_with_compression;

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
}
