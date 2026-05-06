use crate::operator::{PhysicalOperator, PhysicalOperatorKind};

pub fn lower_tier1() -> PhysicalOperator {
    PhysicalOperator::new(PhysicalOperatorKind::Tier1Candidate)
}

pub fn lower_tier2() -> PhysicalOperator {
    PhysicalOperator::new(PhysicalOperatorKind::Tier2Fusion)
}

pub fn lower_tier3() -> PhysicalOperator {
    PhysicalOperator::new(PhysicalOperatorKind::Tier3Exact)
}
