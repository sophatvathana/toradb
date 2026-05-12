use std::collections::HashMap;

use toradb_core::{CandidateSet, DocId};
use toradb_simd::dot_f32;

use crate::graph::csr::CsrGraph;
use crate::sparse::bm25::{Bm25Index, Bm25Snapshot, tokenize};

#[derive(Debug, Clone)]
pub struct IngestDoc {
    pub text: String,
    pub metadata: HashMap<String, String>,
    pub vector: Option<Vec<f32>>,
}

#[derive(Debug)]
pub(crate) struct StoredDoc {
    pub text: String,
    pub metadata: HashMap<String, String>,
    pub vector: Option<Vec<f32>>,
    pub segment: u32,
}

#[derive(Debug, Default)]
pub struct TableCorpus {
    bm25: Bm25Index,
    pub(crate) docs: HashMap<DocId, StoredDoc>,
    graph: CsrGraph,
    next_id: DocId,
    vector_dim: Option<usize>,
}

impl TableCorpus {
    pub fn add(&mut self, doc: IngestDoc, num_segments: u32) -> DocId {
        let id = self.next_id;
        self.next_id += 1;
        let segment = (id % num_segments as u64) as u32;
        if let Some(ref v) = doc.vector {
            match self.vector_dim {
                None => self.vector_dim = Some(v.len()),
                Some(d) if d != v.len() => {}
                _ => {}
            }
        }
        self.bm25.add_document(id, &doc.text);
        if id > 0 {
            self.graph.edges.push((id - 1, id));
            self.graph.edges.push((id, id - 1));
        }
        self.docs.insert(
            id,
            StoredDoc {
                text: doc.text,
                metadata: doc.metadata,
                vector: doc.vector,
                segment,
            },
        );
        id
    }

    pub fn bm25_search(&self, query: &str, k: usize) -> CandidateSet {
        self.bm25.search(query, k)
    }

    pub fn vector_search(&self, query: &[f32], k: usize) -> CandidateSet {
        let mut scored = Vec::new();
        for (&id, doc) in &self.docs {
            let Some(ref v) = doc.vector else { continue };
            if v.len() != query.len() {
                continue;
            }
            scored.push((id, dot_f32(v, query)));
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        let mut out = CandidateSet::with_capacity(scored.len());
        for (id, score) in scored {
            out.push(id, score);
        }
        out
    }

    pub fn metadata_filter(&self, field: &str, value: &str, k: usize) -> CandidateSet {
        let mut out = CandidateSet::with_capacity(k);
        for (&id, doc) in &self.docs {
            if out.len() >= k {
                break;
            }
            if doc.metadata.get(field).map(|v| v == value).unwrap_or(false) {
                out.push(id, 1.0);
            }
        }
        out
    }

    pub fn segment_bm25(&self, query: &str, segment: u32, k: usize) -> CandidateSet {
        let mut scores: Vec<(DocId, f32)> = Vec::new();
        let full = self.bm25.search(query, self.docs.len());
        for (i, id) in full.ids.iter().enumerate() {
            if self.docs.get(id).map(|d| d.segment) == Some(segment) {
                scores.push((*id, full.scores[i]));
            }
        }
        scores.truncate(k);
        let mut out = CandidateSet::with_capacity(scores.len());
        for (id, score) in scores {
            out.push(id, score);
        }
        out
    }

    pub fn neighbors(&self, id: DocId, depth: u32) -> CandidateSet {
        let mut out = CandidateSet::with_capacity(32);
        let mut frontier = vec![id];
        let mut seen = std::collections::HashSet::new();
        seen.insert(id);
        for _ in 0..depth {
            let mut next = Vec::new();
            for &node in &frontier {
                for &(a, b) in &self.graph.edges {
                    let n = if a == node {
                        b
                    } else if b == node {
                        a
                    } else {
                        continue;
                    };
                    if seen.insert(n) {
                        out.push(n, 0.4);
                        next.push(n);
                    }
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn next_id(&self) -> DocId {
        self.next_id
    }

    pub fn doc_text(&self, id: DocId) -> Option<&str> {
        self.docs.get(&id).map(|d| d.text.as_str())
    }

    /// Insert a document with a fixed id (used when reloading columnar segments).
    pub fn add_with_id(&mut self, id: DocId, doc: IngestDoc, num_segments: u32) -> DocId {
        if id >= self.next_id {
            self.next_id = id + 1;
        }
        let segment = (id % num_segments as u64) as u32;
        if let Some(ref v) = doc.vector {
            match self.vector_dim {
                None => self.vector_dim = Some(v.len()),
                Some(d) if d != v.len() => {}
                _ => {}
            }
        }
        self.bm25.add_document(id, &doc.text);
        if id > 0 {
            self.graph.edges.push((id - 1, id));
            self.graph.edges.push((id, id - 1));
        }
        self.docs.insert(
            id,
            StoredDoc {
                text: doc.text,
                metadata: doc.metadata,
                vector: doc.vector,
                segment,
            },
        );
        id
    }

    pub fn insert_stored(&mut self, id: DocId, doc: IngestDoc, num_segments: u32) {
        if id >= self.next_id {
            self.next_id = id + 1;
        }
        let segment = (id % num_segments as u64) as u32;
        if let Some(ref v) = doc.vector {
            match self.vector_dim {
                None => self.vector_dim = Some(v.len()),
                Some(d) if d != v.len() => {}
                _ => {}
            }
        }
        self.docs.insert(
            id,
            StoredDoc {
                text: doc.text,
                metadata: doc.metadata,
                vector: doc.vector,
                segment,
            },
        );
    }

    pub fn rebuild_bm25(&mut self) {
        self.bm25 = Bm25Index::default();
        let mut ids: Vec<DocId> = self.docs.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let text = self.docs.get(&id).map(|d| d.text.as_str()).unwrap_or("");
            self.bm25.add_document(id, text);
        }
    }

    pub fn restore_bm25(&mut self, snap: Bm25Snapshot) {
        self.bm25 = Bm25Index::from_snapshot(snap);
    }

    pub fn bm25_snapshot(&self) -> Bm25Snapshot {
        self.bm25.snapshot()
    }

    pub fn docs_with_ids_since(&self, since_id: DocId) -> Vec<(DocId, IngestDoc)> {
        let mut ids: Vec<DocId> = self
            .docs
            .keys()
            .copied()
            .filter(|id| *id >= since_id)
            .collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| {
                self.docs.get(&id).map(|d| {
                    (
                        id,
                        IngestDoc {
                            text: d.text.clone(),
                            metadata: d.metadata.clone(),
                            vector: d.vector.clone(),
                        },
                    )
                })
            })
            .collect()
    }
}

#[derive(Debug, Default)]
pub struct CorpusStore {
    tables: HashMap<String, TableCorpus>,
}

impl CorpusStore {
    pub fn ensure_table(&mut self, name: &str) -> &mut TableCorpus {
        self.tables.entry(name.to_string()).or_default()
    }

    pub fn table(&self, name: &str) -> Option<&TableCorpus> {
        self.tables.get(name)
    }

    pub fn next_id(&self, table: &str) -> DocId {
        self.table(table).map(|t| t.next_id()).unwrap_or(0)
    }

    pub fn add_documents(&mut self, table: &str, docs: Vec<IngestDoc>, num_segments: u32) -> usize {
        let t = self.ensure_table(table);
        let mut n = 0;
        for doc in docs {
            t.add(doc, num_segments);
            n += 1;
        }
        n
    }

    pub fn add_document_with_id(
        &mut self,
        table: &str,
        id: DocId,
        doc: IngestDoc,
        num_segments: u32,
    ) {
        self.ensure_table(table).add_with_id(id, doc, num_segments);
    }

    pub fn docs_with_ids_since(&self, table: &str, since_id: DocId) -> Vec<(DocId, IngestDoc)> {
        self.table(table)
            .map(|t| t.docs_with_ids_since(since_id))
            .unwrap_or_default()
    }

    pub fn all_documents(&self, table: &str) -> Vec<(DocId, IngestDoc)> {
        self.docs_with_ids_since(table, 0)
    }

    pub fn insert_stored(&mut self, table: &str, id: DocId, doc: IngestDoc, num_segments: u32) {
        self.ensure_table(table).insert_stored(id, doc, num_segments);
    }

    pub fn rebuild_bm25(&mut self, table: &str) {
        self.ensure_table(table).rebuild_bm25();
    }

    pub fn restore_bm25(&mut self, table: &str, snap: Bm25Snapshot) {
        self.ensure_table(table).restore_bm25(snap);
    }

    pub fn bm25_snapshot(&self, table: &str) -> Option<Bm25Snapshot> {
        self.table(table).map(|t| t.bm25_snapshot())
    }

    pub fn expand_query_terms(&self, table: &str, query: &str) -> String {
        let Some(t) = self.table(table) else {
            return query.to_string();
        };
        let mut terms: Vec<String> = tokenize(query);
        let sample = t.bm25_search(query, 3);
        for id in sample.ids {
            if let Some(text) = t.doc_text(id) {
                for term in tokenize(text) {
                    if term.len() > 3 && !terms.contains(&term) {
                        terms.push(term);
                    }
                }
            }
        }
        terms.join(" ")
    }
}
