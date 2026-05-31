use serde::{Deserialize, Serialize};

use crate::schema::DocId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredDoc {
    pub id: DocId,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropRecord {
    pub id: DocId,
    pub stage: DropStage,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropStage {
    MetadataFilter,
    Tier1BudgetCut,
    Tier2BudgetCut,
    CragFilter,
    Tier3BudgetCut,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TierTrace {
    pub bm25_candidates: Vec<ScoredDoc>,
    pub hnsw_candidates: Vec<ScoredDoc>,
    pub rrf_merged: Vec<ScoredDoc>,
    pub drops: Vec<DropRecord>,
    pub latency_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceRecord {
    pub query: String,
    pub strategy: Option<String>,
    pub tier1: TierTrace,
    pub tier2: TierTrace,
    pub tier3: TierTrace,
    pub final_ids: Vec<DocId>,
    pub total_latency_ms: f64,
}

/// Zero-cost when disabled: only allocates when `enabled = true`.
#[derive(Debug, Default, Clone)]
pub struct ProvenanceCollector {
    pub enabled: bool,
    pub record: ProvenanceRecord,
}

impl ProvenanceCollector {
    pub fn new(enabled: bool, query: String, strategy: Option<String>) -> Self {
        Self {
            enabled,
            record: ProvenanceRecord {
                query,
                strategy,
                tier1: TierTrace::default(),
                tier2: TierTrace::default(),
                tier3: TierTrace::default(),
                final_ids: Vec::new(),
                total_latency_ms: 0.0,
            },
        }
    }

    pub fn record_bm25(&mut self, id: DocId, score: f32) {
        if self.enabled {
            self.record.tier1.bm25_candidates.push(ScoredDoc { id, score });
        }
    }

    pub fn record_hnsw(&mut self, id: DocId, score: f32) {
        if self.enabled {
            self.record.tier1.hnsw_candidates.push(ScoredDoc { id, score });
        }
    }

    pub fn record_rrf(&mut self, id: DocId, score: f32) {
        if self.enabled {
            self.record.tier2.rrf_merged.push(ScoredDoc { id, score });
        }
    }

    pub fn record_drop(&mut self, id: DocId, stage: DropStage, reason: String) {
        if self.enabled {
            let drop = DropRecord { id, stage, reason };
            self.record.tier2.drops.push(drop);
        }
    }

    pub fn record_metadata_drop(&mut self, id: DocId, reason: String) {
        if self.enabled {
            self.record.tier1.drops.push(DropRecord {
                id,
                stage: DropStage::MetadataFilter,
                reason,
            });
        }
    }

    pub fn record_tier1_drop(&mut self, id: DocId, reason: String) {
        if self.enabled {
            self.record.tier1.drops.push(DropRecord {
                id,
                stage: DropStage::Tier1BudgetCut,
                reason,
            });
        }
    }

    pub fn record_tier3_drop(&mut self, id: DocId, reason: String) {
        if self.enabled {
            self.record.tier3.drops.push(DropRecord {
                id,
                stage: DropStage::Tier3BudgetCut,
                reason,
            });
        }
    }

    pub fn record_tier_latency(&mut self, tier: u8, micros: u64) {
        if self.enabled {
            match tier {
                1 => self.record.tier1.latency_us = micros,
                2 => self.record.tier2.latency_us = micros,
                3 => self.record.tier3.latency_us = micros,
                _ => {}
            }
        }
    }

    pub fn set_final(&mut self, ids: &[DocId]) {
        if self.enabled {
            self.record.final_ids = ids.to_vec();
        }
    }

    pub fn set_total_latency_ms(&mut self, ms: f64) {
        if self.enabled {
            self.record.total_latency_ms = ms;
        }
    }

    pub fn finish(self) -> Option<ProvenanceRecord> {
        if self.enabled {
            Some(self.record)
        } else {
            None
        }
    }
}

impl Default for ProvenanceRecord {
    fn default() -> Self {
        Self {
            query: String::new(),
            strategy: None,
            tier1: TierTrace::default(),
            tier2: TierTrace::default(),
            tier3: TierTrace::default(),
            final_ids: Vec::new(),
            total_latency_ms: 0.0,
        }
    }
}
