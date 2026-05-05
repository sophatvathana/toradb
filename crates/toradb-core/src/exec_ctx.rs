/// Per-query execution budgets for tiered retrieval.
#[derive(Debug, Clone, Copy)]
pub struct ExecCtx {
    pub tier1_budget: u32,
    pub tier2_budget: u32,
    pub tier3_budget: u32,
}

impl ExecCtx {
    pub fn new(tier1_budget: u32, tier2_budget: u32, tier3_budget: u32) -> Self {
        Self {
            tier1_budget,
            tier2_budget,
            tier3_budget,
        }
    }

    pub fn default_retrieval() -> Self {
        Self::new(1000, 100, 20)
    }
}

impl Default for ExecCtx {
    fn default() -> Self {
        Self::default_retrieval()
    }
}
