use crate::{
    error::AppError,
    storage::{
        model::{Stage, UpdatePlan, UpdateTransaction},
        Store,
    },
};
pub fn run(
    store: &mut Store,
    plan_id: &str,
    continue_after_failure: bool,
    mut execute: impl FnMut(&mut Store, &str) -> Result<UpdateTransaction, AppError>,
) -> Result<UpdatePlan, AppError> {
    let plan = store.plan(plan_id)?;
    if plan.state == "running"
        || plan.transactions.iter().any(|t| {
            matches!(
                t.stage,
                Stage::Preflight
                    | Stage::BackingUp
                    | Stage::Prepared
                    | Stage::Installing
                    | Stage::Verifying
                    | Stage::Recovering
            )
        })
    {
        return Err(AppError::new(
            "PLAN_INTERRUPTED",
            "此计划有未完成事务，请先检查恢复。",
        ));
    }
    if plan.cancel_requested {
        store.cancel_queued(plan_id)?;
        return store.plan(plan_id);
    }
    if plan.state == "paused" && !continue_after_failure {
        return Err(AppError::new(
            "PLAN_PAUSED",
            "计划已暂停，请明确选择继续剩余项或结束。",
        ));
    }
    if ["finished", "cancelled"].contains(&plan.state.as_str()) {
        return store.plan(plan_id);
    }
    store.set_plan_state(plan_id, "running")?;
    for tx in plan
        .transactions
        .into_iter()
        .filter(|t| t.stage == Stage::Queued)
    {
        if store.cancel_requested(plan_id)? {
            store.cancel_queued(plan_id)?;
            return store.plan(plan_id);
        }
        match execute(store, &tx.id) {
            Ok(result) if result.stage == Stage::Succeeded => {}
            Ok(_) | Err(_) => {
                if store.cancel_requested(plan_id)? {
                    store.cancel_queued(plan_id)?;
                } else {
                    store.set_plan_state(plan_id, "paused")?;
                }
                return store.plan(plan_id);
            }
        }
    }
    if store.cancel_requested(plan_id)? {
        store.cancel_queued(plan_id)?;
    } else {
        store.set_plan_state(plan_id, "finished")?;
    }
    store.plan(plan_id)
}
