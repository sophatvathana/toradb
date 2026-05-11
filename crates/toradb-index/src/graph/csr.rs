use toradb_core::DocId;

#[derive(Debug, Default, Clone)]
pub struct CsrGraph {
    pub edges: Vec<(DocId, DocId)>,
}

impl CsrGraph {
    pub fn neighbors(&self, node: DocId) -> impl Iterator<Item = DocId> + '_ {
        self.edges.iter().filter_map(move |(a, b)| {
            if *a == node {
                Some(*b)
            } else if *b == node {
                Some(*a)
            } else {
                None
            }
        })
    }
}
