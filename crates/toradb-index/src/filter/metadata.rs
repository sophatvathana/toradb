use toradb_core::CandidateSet;

use crate::corpus::CorpusStore;

pub fn parse_field_filter(query: &str) -> Option<(&str, &str)> {
    for word in query.split_whitespace() {
        if let Some((k, v)) = word.split_once(':') {
            if !k.is_empty() && !v.is_empty() {
                return Some((k, v));
            }
        }
    }
    None
}

pub fn filter(store: &CorpusStore, table: &str, query: &str, k: usize) -> CandidateSet {
    let Some((field, value)) = parse_field_filter(query) else {
        return CandidateSet::default();
    };
    store
        .table(table)
        .map(|t| t.metadata_filter(field, value, k))
        .unwrap_or_default()
}
