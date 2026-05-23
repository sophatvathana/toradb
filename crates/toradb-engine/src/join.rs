use toradb_core::CandidateSet;
use toradb_sql::ast::JoinClause;

use crate::dag::DagRunner;

pub fn apply_metadata_join(
    dag: &mut DagRunner,
    left_table: &str,
    join: &JoinClause,
    candidates: &mut CandidateSet,
) -> Result<(), String> {
    dag.ensure_table(left_table);
    dag.ensure_table(&join.right_table);

    let store = &dag.retrieval.store;
    let mut kept = CandidateSet::with_capacity(candidates.len());
    for (i, &id) in candidates.ids.iter().enumerate() {
        let Some(left_val) = store.doc_metadata_value(left_table, id, &join.left_key) else {
            continue;
        };
        if store.any_doc_metadata_eq(&join.right_table, &join.right_key, left_val) {
            kept.push(id, candidates.scores[i]);
        }
    }
    *candidates = kept;
    Ok(())
}
