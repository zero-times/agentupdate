use crate::{
    backup,
    error::AppError,
    platform::locks::OperationLock,
    storage::{model::Stage, Store},
};
use serde::Serialize;
use std::{fs, path::Path};
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryItem {
    pub transaction_id: String,
    pub stage: Stage,
    pub material_ready: bool,
    pub material_error: Option<String>,
    pub current_version: Option<String>,
    pub installation_error: Option<String>,
}
#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryReport {
    pub busy: bool,
    pub items: Vec<RecoveryItem>,
}
pub fn reconcile(store: &mut Store, home: &Path) -> Result<RecoveryReport, AppError> {
    let _lock = match OperationLock::acquire(&store.root.join("locks/operation.lock")) {
        Ok(lock) => lock,
        Err(e) if e.code == "OPERATION_BUSY" => {
            return Ok(RecoveryReport {
                busy: true,
                items: vec![],
            })
        }
        Err(e) => return Err(e),
    };
    let mut report = RecoveryReport::default();
    for plan in store.plans()? {
        let mut occupied = false;
        for tx in plan.transactions {
            let next = match tx.stage {
                Stage::Preflight | Stage::BackingUp | Stage::Prepared => {
                    Some(Stage::PreparationFailed)
                }
                Stage::Installing | Stage::Verifying => Some(Stage::NeedsRecovery),
                Stage::Recovering => Some(Stage::RecoveryFailed),
                _ => None,
            };
            if next.is_none() && !matches!(tx.stage, Stage::NeedsRecovery | Stage::RecoveryFailed) {
                continue;
            }
            let _instance_lock = match tx
                .installation
                .prefix
                .as_ref()
                .map(|p| OperationLock::npm(p))
            {
                Some(Ok(lock)) => Some(lock),
                Some(Err(e)) if e.code == "OPERATION_BUSY" => {
                    occupied = true;
                    report.busy = true;
                    continue;
                }
                _ => None,
            };
            let tx = if let Some(next) = next {
                store.transition(
                    &tx.id,
                    tx.stage,
                    next,
                    Some(if next == Stage::PreparationFailed {
                        "INTERRUPTED_BEFORE_INSTALL"
                    } else {
                        "INTERRUPTED_MUTATION"
                    }),
                )?
            } else {
                tx
            };
            let material = backup::load(store, &tx);
            let current = (|| {
                let resolved = fs::canonicalize(&tx.installation.command_path).map_err(|_| {
                    AppError::new("COMMAND_UNAVAILABLE", "当前命令缺失或链接损坏。")
                })?;
                let package =
                    tx.installation.package_root.as_ref().ok_or_else(|| {
                        AppError::new("INSTALLATION_CHANGED", "安装范围无法核验。")
                    })?;
                if !resolved.starts_with(package) {
                    return Err(AppError::new(
                        "INSTALLATION_CHANGED",
                        "当前命令已离开原安装范围。",
                    ));
                }
                super::native::verify(&tx.installation, &resolved, home, true).map(|v| v.0)
            })();
            report.items.push(RecoveryItem {
                transaction_id: tx.id,
                stage: tx.stage,
                material_ready: material.is_ok(),
                material_error: material.err().map(|e| e.code.into()),
                current_version: current.as_ref().ok().cloned(),
                installation_error: current.err().map(|e| e.code.into()),
            });
        }
        if !occupied && plan.state == "running" {
            store.set_plan_state(&plan.id, "paused")?;
        }
        if !occupied && plan.cancel_requested {
            store.cancel_queued(&plan.id)?;
        }
    }
    Ok(report)
}
