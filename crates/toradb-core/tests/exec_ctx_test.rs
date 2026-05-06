use toradb_core::ExecCtx;

#[test]
fn exec_ctx_respects_budgets() {
    let ctx = ExecCtx::new(500, 50, 10);
    assert_eq!(ctx.tier1_budget, 500);
    assert_eq!(ctx.tier2_budget, 50);
    assert_eq!(ctx.tier3_budget, 10);
}
