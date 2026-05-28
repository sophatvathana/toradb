use std::path::Path;

use toradb_core::{Batch, ExecCtx};
use toradb_index::CorpusStore;
use toradb_simd::dot_f32;
use toradb_storage::StorageCaches;

use crate::operator::{PhysicalOperator, PhysicalOperatorKind};
use crate::persist;

pub fn lower_tier1() -> PhysicalOperator {
    PhysicalOperator::new(PhysicalOperatorKind::Tier1Candidate)
}

pub fn lower_tier2() -> PhysicalOperator {
    PhysicalOperator::new(PhysicalOperatorKind::Tier2Fusion)
}

pub fn lower_tier3() -> PhysicalOperator {
    PhysicalOperator::new(PhysicalOperatorKind::Tier3Exact)
}

/// Exact rerank: rescore top tier-2 candidates with full-precision vectors when available.
pub fn tier3_rescore(
    base: &Path,
    batch: &mut Batch,
    ctx: &ExecCtx,
    _caches: &mut StorageCaches,
    store: &CorpusStore,
) {
    let Some(query_vec) = batch.query_vector.as_ref() else {
        return;
    };
    if query_vec.is_empty() || batch.candidates.is_empty() {
        return;
    }
    let table = &batch.table;
    let cap = ctx.tier3_budget as usize;
    let docs = store.documents_by_ids(table, &batch.candidates.ids);
    let doc_map: std::collections::HashMap<_, _> = docs.into_iter().collect();
    let mut rescored = 0usize;
    for (i, id) in batch.candidates.ids.iter().enumerate() {
        if let Some(doc) = doc_map.get(id) {
            if let Some(ref v) = doc.vector {
                batch.candidates.scores[i] = dot_f32(query_vec, v);
                rescored += 1;
            }
        }
    }
    if rescored == 0 && persist::table_has_quant_sidecars(base, table).unwrap_or(false) {
        if let Ok(vecs) = persist::load_vectors_for_ids(base, table, &batch.candidates.ids) {
            for (i, id) in batch.candidates.ids.iter().enumerate() {
                if let Some(v) = vecs.get(id) {
                    batch.candidates.scores[i] = dot_f32(query_vec, v);
                }
            }
        }
    }
    batch.candidates.sort_by_score(true);
    batch.candidates.truncate(cap);
}
