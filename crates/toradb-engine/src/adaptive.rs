use toradb_core::ExecCtx;

/// Adjust tier budgets from query shape and retrieval strategy (stub auto-tuner).
pub fn tune_ctx(base: ExecCtx, query: &str, strategy: Option<&str>) -> ExecCtx {
    let mut ctx = base;
    let tokens = query.split_whitespace().count().max(1) as u32;
    let extra = (tokens.saturating_mul(8)).min(400);
    ctx.tier1_budget = ctx.tier1_budget.saturating_add(extra / 2);
    match strategy {
        Some("sparse") | Some("bm25") => {
            ctx.tier2_budget = ctx.tier2_budget.saturating_mul(3) / 4;
        }
        Some("dense") | Some("vector") | Some("hnsw") | Some("diskann") | Some("ann") => {
            ctx.tier1_budget = ctx.tier1_budget.saturating_mul(2).min(2000);
        }
        Some("graph") | Some("hybrid") => {
            ctx.tier2_budget = ctx.tier2_budget.saturating_mul(2).min(500);
        }
        Some("hyde") | Some("crag") => {
            ctx.tier2_budget = ctx.tier2_budget.saturating_mul(3) / 2;
        }
        _ => {}
    }
    ctx
}
