use toradb_core::{Batch, ExecCtx};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalOperatorKind {
    Tier1Candidate,
    Tier2Fusion,
    Tier3Exact,
    Materialize,
}

#[derive(Debug)]
pub struct PhysicalOperator {
    pub kind: PhysicalOperatorKind,
}

impl PhysicalOperator {
    pub fn new(kind: PhysicalOperatorKind) -> Self {
        Self { kind }
    }

    pub fn execute(&self, batch: &mut Batch, ctx: &ExecCtx) -> usize {
        let budget = match self.kind {
            PhysicalOperatorKind::Tier1Candidate => ctx.tier1_budget as usize,
            PhysicalOperatorKind::Tier2Fusion => ctx.tier2_budget as usize,
            PhysicalOperatorKind::Tier3Exact => ctx.tier3_budget as usize,
            PhysicalOperatorKind::Materialize => ctx.tier3_budget as usize,
        };
        batch.candidates.truncate(budget);
        batch.candidates.len()
    }
}
