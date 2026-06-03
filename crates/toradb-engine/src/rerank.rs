use std::collections::HashMap;

use toradb_core::{
    parse_timestamp_millis, CandidateSet, DecaySpec, ProvenanceCollector, ScoreBreakdown,
};

use crate::dag::DagRunner;

pub fn knobs_active(field_boosts: &HashMap<String, f32>, decay: &Option<DecaySpec>) -> bool {
    !field_boosts.is_empty() || decay.is_some()
}

pub fn apply_ranking_knobs(
    dag: &mut DagRunner,
    table: &str,
    candidates: &mut CandidateSet,
    field_boosts: &HashMap<String, f32>,
    decay: &Option<DecaySpec>,
    now_unix_millis: i64,
    prov: Option<&mut ProvenanceCollector>,
) -> Result<(), String> {
    if candidates.ids.is_empty() || !knobs_active(field_boosts, decay) {
        return Ok(());
    }

    let docs: HashMap<u64, toradb_index::IngestDoc> = dag
        .fetch_documents(table, &candidates.ids)?
        .into_iter()
        .collect();

    apply_ranking_knobs_with_docs(candidates, &docs, field_boosts, decay, now_unix_millis, prov);
    Ok(())
}

pub fn apply_ranking_knobs_with_docs(
    candidates: &mut CandidateSet,
    docs: &HashMap<u64, toradb_index::IngestDoc>,
    field_boosts: &HashMap<String, f32>,
    decay: &Option<DecaySpec>,
    now_unix_millis: i64,
    mut prov: Option<&mut ProvenanceCollector>,
) {
    if candidates.ids.is_empty() || !knobs_active(field_boosts, decay) {
        return;
    }

    let mut breakdown: Vec<ScoreBreakdown> = Vec::with_capacity(candidates.ids.len());
    for i in 0..candidates.ids.len() {
        let id = candidates.ids[i];
        let base = candidates.scores[i];
        let meta = docs.get(&id).map(|d| &d.metadata);

        let mut boost = 1.0f32;
        if let Some(meta) = meta {
            for (field, &factor) in field_boosts {
                let has = meta.get(field).is_some_and(|v| !v.trim().is_empty());
                if has {
                    boost *= factor;
                }
            }
        }

        let mut decay_factor = 1.0f32;
        if let (Some(spec), Some(meta)) = (decay.as_ref(), meta) {
            if let Some(ts) = meta
                .get(&spec.field)
                .and_then(|v| parse_timestamp_millis(v))
            {
                let age_days = (now_unix_millis - ts) as f32 / 86_400_000.0;
                if age_days > 0.0 && spec.half_life_days > 0.0 {
                    decay_factor = 0.5f32.powf(age_days / spec.half_life_days);
                }
            }
        }

        let final_score = base * boost * decay_factor;
        candidates.scores[i] = final_score;
        breakdown.push(ScoreBreakdown {
            id,
            base,
            boost,
            decay: decay_factor,
            final_score,
        });
    }

    candidates.sort_by_score(true);

    if let Some(p) = prov.as_mut() {
        p.set_score_breakdown(breakdown);
    }
}
