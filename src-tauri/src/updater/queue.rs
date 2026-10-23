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

/// Run up to four independent tool updates. Each worker owns a SQLite connection;
/// shared agent/configuration and installation-prefix locks serialize conflicting work.
pub fn run_concurrent(
    root: &std::path::Path,
    plan_id: &str,
    execute: impl Fn(&std::path::Path, &str) -> Result<UpdateTransaction, AppError> + Sync,
) -> Result<UpdatePlan, AppError> {
    use crate::platform::locks::OperationLock;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut store = Store::open(root)?;
    let plan = store.plan(plan_id)?;
    if plan.state == "running"
        || plan.transactions.iter().any(|tx| {
            matches!(
                tx.stage,
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
            "计划正在执行或已中断，请检查任务记录。",
        ));
    }
    if plan.cancel_requested {
        store.cancel_queued(plan_id)?;
        return store.plan(plan_id);
    }
    if ["finished", "cancelled"].contains(&plan.state.as_str()) {
        return Ok(plan);
    }
    let pending: Vec<_> = plan
        .transactions
        .into_iter()
        .filter(|tx| tx.stage == Stage::Queued)
        .collect();
    store.set_plan_state(plan_id, "running")?;
    let next = AtomicUsize::new(0);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..pending.len().min(4))
            .map(|_| {
                scope.spawn(|| -> Result<(), AppError> {
                    let mut worker = Store::open(root)?;
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(tx) = pending.get(index) else {
                            break;
                        };
                        if worker.cancel_requested(plan_id)? {
                            break;
                        }
                        let key = format!("cli:{}", tx.installation.agent_id.key());
                        let _agent = OperationLock::resource(root, &key, true)?;
                        if worker.cancel_requested(plan_id)? {
                            break;
                        }
                        // Panics and early failures must not leave a queued item in a finished plan.
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            execute(root, &tx.id)
                        }));
                        let code = match result {
                            Ok(Ok(ref done))
                                if done.stage == Stage::Succeeded
                                    || done.stage == Stage::Cancelled =>
                            {
                                None
                            }
                            Ok(Err(ref e)) => Some(e.code),
                            _ => Some("UPDATE_FAILED"),
                        };
                        if let Some(code) = code {
                            let current = worker.transaction(&tx.id)?;
                            if current.stage == Stage::Queued {
                                worker.transition(&tx.id, Stage::Queued, Stage::Preflight, None)?;
                                worker.transition(
                                    &tx.id,
                                    Stage::Preflight,
                                    Stage::PreparationFailed,
                                    Some(code),
                                )?;
                            } else if matches!(
                                current.stage,
                                Stage::Preflight | Stage::BackingUp | Stage::Prepared
                            ) {
                                worker.transition(
                                    &tx.id,
                                    current.stage,
                                    Stage::PreparationFailed,
                                    Some(code),
                                )?;
                            } else if matches!(current.stage, Stage::Installing | Stage::Verifying)
                            {
                                worker.transition(
                                    &tx.id,
                                    current.stage,
                                    Stage::NeedsRecovery,
                                    Some(code),
                                )?;
                            }
                        }
                    }
                    Ok(())
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    Err(AppError::new(
                        "QUEUE_INTERRUPTED",
                        "工作线程中断，请查看任务记录。",
                    ))
                })
            })
            .collect::<Vec<_>>()
    });
    for result in results {
        if let Err(e) = result {
            store.set_plan_state(plan_id, "paused")?;
            return Err(e);
        }
    }
    if store.cancel_requested(plan_id)? {
        store.cancel_queued(plan_id)?;
    } else {
        store.set_plan_state(plan_id, "finished")?;
    }
    store.plan(plan_id)
}
