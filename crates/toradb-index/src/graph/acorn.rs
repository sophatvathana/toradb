use toradb_core::CandidateSet;

use crate::corpus::CorpusStore;
use crate::filter::metadata::parse_field_filter;

pub fn refine(
    store: &CorpusStore,
    table: &str,
    candidates: &CandidateSet,
    query: &str,
    cap: usize,
) -> CandidateSet {
    let mut out = CandidateSet::with_capacity(cap.min(candidates.len()));
    let filter = parse_field_filter(query);
    let Some(t) = store.table(table) else {
        return out;
    };
    for (i, id) in candidates.ids.iter().enumerate() {
        if out.len() >= cap {
            break;
        }
        let score = candidates.scores[i];
        if let Some((field, value)) = filter {
            let keep = t
                .docs
                .get(id)
                .and_then(|d| d.metadata.get(field))
                .map(|v| v == value)
                .unwrap_or(false);
            if keep {
                out.push(*id, score);
            }
        } else if score > 0.0 {
            out.push(*id, score);
        }
    }
    out
}
