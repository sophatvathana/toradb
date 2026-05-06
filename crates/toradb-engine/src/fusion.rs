use toradb_core::CandidateSet;

pub fn rrf_merge(a: &CandidateSet, b: &CandidateSet, k: u32) -> CandidateSet {
    let mut out = CandidateSet::with_capacity(a.len() + b.len());
    for (i, id) in a.ids.iter().enumerate() {
        let score = 1.0 / (k as f32 + i as f32 + 1.0);
        out.push(*id, score);
    }
    for (i, id) in b.ids.iter().enumerate() {
        let score = 1.0 / (k as f32 + i as f32 + 1.0);
        if let Some(pos) = out.ids.iter().position(|x| x == id) {
            out.scores[pos] += score;
        } else {
            out.push(*id, score);
        }
    }
    out
}
