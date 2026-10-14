pub mod native;
pub mod npm;
pub mod queue;
pub mod recovery;
pub mod rollback;
use crate::{
    backup,
    error::AppError,
    storage::{
        model::{Stage, UpdateTransaction},
        Store,
    },
};
use serde::Serialize;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub state: &'static str,
}
pub fn status() -> UpdateStatus {
    UpdateStatus { state: "available" }
}
pub fn execute_item(
    store: &mut Store,
    id: &str,
    context: &npm::Context,
) -> Result<UpdateTransaction, AppError> {
    let target = store.transaction(id)?.installation.id;
    crate::progress::operation(&target, || execute_item_inner(store, id, context))
}
fn execute_item_inner(
    store: &mut Store,
    id: &str,
    context: &npm::Context,
) -> Result<UpdateTransaction, AppError> {
    let tx = store.transaction(id)?;
    let prefix = tx
        .installation
        .prefix
        .as_ref()
        .ok_or_else(|| AppError::new("UNSUPPORTED_INSTALLATION", "实例没有标准 npm 目录。"))?;
    let _lock = crate::platform::locks::OperationLock::npm(prefix)?;
    let check_cancel = |store: &Store| -> Result<(), AppError> {
        if store.cancel_requested(&tx.plan_id)? {
            Err(AppError::new("CANCELLED", "更新已在修改安装前安全取消。"))
        } else {
            Ok(())
        }
    };
    if check_cancel(store).is_err() {
        crate::progress::phase(crate::progress::Phase::Cancelled);
        return store.transition(id, Stage::Queued, Stage::Cancelled, None);
    }
    let transaction = store.transition(id, Stage::Queued, Stage::Preflight, None)?;
    let preparation = (|| {
        if native::handles(&transaction.installation) {
            native::preflight(&transaction.installation, context)?;
        } else {
            npm::preflight(&transaction.installation, context)?;
        }
        check_cancel(store)?;
        let tx = store.transition(id, Stage::Preflight, Stage::BackingUp, None)?;
        crate::progress::phase(crate::progress::Phase::BackingUp);
        backup::prepare(store, &tx, &context.home)?;
        check_cancel(store)?;
        let work = store.root.join("work").join(id);
        let tx = store.transaction(id)?;
        let staged = if native::handles(&tx.installation) {
            native::stage(&tx, context, &work)?
        } else {
            npm::stage(&tx, context, &work)?
        };
        // Recheck original identity and configuration immediately before changing the installation.
        if native::handles(&transaction.installation) {
            native::preflight(&transaction.installation, context)?;
        } else {
            npm::preflight(&transaction.installation, context)?;
        }
        let (directory, manifest) = backup::load(store, &store.transaction(id)?)?;
        let original_package = backup::snapshot::scan(
            transaction.installation.package_root.as_ref().unwrap(),
            None,
        )?;
        if original_package.entries != manifest.package.entries {
            return Err(AppError::new(
                "INSTALLATION_CHANGED",
                "备份后安装文件已变化，请结束此计划并重新准备。",
            ));
        }
        let current =
            backup::config_snapshot(&manifest.config_root, transaction.installation.agent_id)?;
        if current.entries != manifest.config.entries {
            return Err(AppError::new(
                "CONFIG_CHANGED",
                "备份后配置已变化，请结束此计划并重新准备。",
            ));
        }
        backup::verify_material(&directory, &manifest)?;
        check_cancel(store)?;
        Ok(staged)
    })();
    let staged = match preparation {
        Ok(staged) => staged,
        Err(error) => {
            let current = store.transaction(id)?;
            store.transition(
                id,
                current.stage,
                if error.code == "CANCELLED" {
                    Stage::Cancelled
                } else {
                    Stage::PreparationFailed
                },
                Some(error.code),
            )?;
            return Err(error);
        }
    };
    let transaction = store.transition(id, Stage::Prepared, Stage::Installing, None)?;
    crate::progress::phase(crate::progress::Phase::Installing);
    let outcome = (|| {
        if native::handles(&transaction.installation) {
            native::install(&transaction, &staged, context)?;
        } else {
            npm::swap_package(&transaction, &staged)?;
        }
        let transaction = store.transition(id, Stage::Installing, Stage::Verifying, None)?;
        crate::progress::phase(crate::progress::Phase::Verifying);
        if native::handles(&transaction.installation) {
            native::verify_installed(&transaction, context, &store.root.join("work").join(id))
        } else {
            npm::verify_installed(&transaction, context)
        }
    })();
    match outcome {
        Ok(()) => store.transition(id, Stage::Verifying, Stage::Succeeded, None),
        Err(error) => {
            let current = store.transaction(id)?;
            store.transition(id, current.stage, Stage::NeedsRecovery, Some(error.code))?;
            Err(error)
        }
    }
}
