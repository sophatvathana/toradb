use toradb_core::Batch;

/// Corrective retrieval: drop candidates below the median score.
pub fn apply_crag(batch: &mut Batch) {
    if batch.candidates.len() <= 1 {
        return;
    }
    let mut scores = batch.candidates.scores.clone();
    scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = scores[scores.len() / 2];
    let mut kept_ids = Vec::new();
    let mut kept_scores = Vec::new();
    for (i, id) in batch.candidates.ids.iter().enumerate() {
        if batch.candidates.scores[i] >= median {
            kept_ids.push(*id);
            kept_scores.push(batch.candidates.scores[i]);
        }
    }
    if !kept_ids.is_empty() {
        batch.candidates.ids = kept_ids;
        batch.candidates.scores = kept_scores;
    }
}
