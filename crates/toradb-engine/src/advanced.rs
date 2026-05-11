use toradb_core::Batch;

/// Hypothetical-document expansion hook (tier-2); production encoder deferred.
pub fn apply_hyde(batch: &mut Batch) {
    if batch.query.is_empty() {
        return;
    }
    batch.query = format!("{} [hyde-expanded]", batch.query);
}

/// Corrective retrieval hook: trim low-confidence candidates (stub threshold).
pub fn apply_crag(batch: &mut Batch) {
    if batch.candidates.len() <= 1 {
        return;
    }
    let keep = (batch.candidates.len() / 2).max(1);
    batch.candidates.truncate(keep);
}
