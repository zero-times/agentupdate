use crate::{agents, backup, error::AppError, platform, releases, storage, updater};
use serde::Serialize;

#[derive(Default)]
pub struct AppState {
    pub restarting: std::sync::atomic::AtomicBool,
    pub installations: std::sync::Mutex<Vec<agents::discovery::Installation>>,
    pub store: std::sync::Mutex<Option<storage::Store>>,
    pub operation: std::sync::Mutex<()>,
}

pub fn execute(state: &AppState, id: &str) -> Result<storage::model::UpdateTransaction, AppError> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| AppError::new("OPERATION_BUSY", "已有更新或恢复操作正在进行。"))?;
    let (root, transaction) = with_store(state, |store| {
        Ok((store.root.clone(), store.transaction(id)?))
    })?;
    let _lock = platform::locks::OperationLock::acquire(&root.join("locks/operation.lock"))?;
    let context = updater::npm::Context::current(&transaction.installation)?;
    updater::execute_item(&mut storage::Store::open(&root)?, id, &context)
}

pub fn execute_plan(
    state: &AppState,
    id: &str,
    continue_after_failure: bool,
) -> Result<storage::model::UpdatePlan, AppError> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| AppError::new("OPERATION_BUSY", "已有更新或恢复操作正在进行。"))?;
    let root = with_store(state, |store| Ok(store.root.clone()))?;
    let _lock = platform::locks::OperationLock::acquire(&root.join("locks/operation.lock"))?;
    updater::queue::run(
        &mut storage::Store::open(&root)?,
        id,
        continue_after_failure,
        |store, id| {
            let tx = store.transaction(id)?;
            let context = match updater::npm::Context::current(&tx.installation) {
                Ok(context) => context,
                Err(e) => {
                    store.transition(
                        id,
                        storage::model::Stage::Queued,
                        storage::model::Stage::Preflight,
                        None,
                    )?;
                    store.transition(
                        id,
                        storage::model::Stage::Preflight,
                        storage::model::Stage::PreparationFailed,
                        Some(e.code),
                    )?;
                    return Err(e);
                }
            };
            #[cfg(feature = "desktop-validation")]
            if let Some(root) = std::env::var_os("AGENTUPDATE_TEST_CATALOG_DIR") {
                let source = std::path::PathBuf::from(root)
                    .join(format!("{}.tgz", tx.installation.agent_id.key()));
                let work = store.root.join("work").join(id);
                platform::files::private_directory(&work)?;
                std::fs::copy(source, work.join("target.tgz"))
                    .map_err(|_| AppError::new("ARTIFACT_READ", "隔离测试制品不可用。"))?;
            }
            updater::execute_item(store, id, &context)
        },
    )
}

pub fn rollback(
    state: &AppState,
    id: &str,
    confirm_changed_config: bool,
) -> Result<storage::model::UpdateTransaction, AppError> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| AppError::new("OPERATION_BUSY", "已有更新或恢复操作正在进行。"))?;
    let root = with_store(state, |store| Ok(store.root.clone()))?;
    let _lock = platform::locks::OperationLock::acquire(&root.join("locks/operation.lock"))?;
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| AppError::new("HOME_UNAVAILABLE", "无法确认用户配置目录。"))?;
    let context = updater::npm::Context {
        home,
        npm: std::path::PathBuf::new(),
        cache: None,
        offline: true,
    };
    updater::rollback::execute(
        &mut storage::Store::open(&root)?,
        id,
        &context,
        confirm_changed_config,
    )
}

pub fn with_store<T>(
    state: &AppState,
    action: impl FnOnce(&mut storage::Store) -> Result<T, AppError>,
) -> Result<T, AppError> {
    if state.restarting.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(AppError::new(
            "APP_RESTARTING",
            "应用正在安装更新并重启，请稍候。",
        ));
    }
    let mut guard = state
        .store
        .lock()
        .map_err(|_| AppError::new("STATE_UNAVAILABLE", "事务记录暂不可用。"))?;
    if guard.is_none() {
        let root = platform::files::default_data_root()?;
        #[cfg(feature = "desktop-validation")]
        let root = std::env::var_os("AGENTUPDATE_TEST_DATA_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or(root);
        *guard = Some(storage::Store::open(&root)?);
    }
    action(guard.as_mut().unwrap())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanRequest {
    pub instance_id: String,
    pub target: String,
    pub allow_prerelease: bool,
}

pub fn build_plan(
    state: &AppState,
    requests: Vec<PlanRequest>,
) -> Result<storage::model::UpdatePlan, AppError> {
    if requests.is_empty() || requests.len() > 32 || requests.iter().any(|r| r.target.len() > 128) {
        return Err(AppError::new(
            "INVALID_PLAN",
            "请选择 1 至 32 个实例和有效目标版本。",
        ));
    }
    let instances = state
        .installations
        .lock()
        .map_err(|_| AppError::new("STATE_UNAVAILABLE", "实例信息暂不可用。"))?
        .clone();
    let mut catalogs = std::collections::HashMap::new();
    let mut items = Vec::new();
    for request in requests {
        let installation = instances
            .iter()
            .find(|i| i.id == request.instance_id && i.supported && i.version_verified)
            .cloned()
            .ok_or_else(|| AppError::new("INSTANCE_UNAVAILABLE", "请重新扫描并选择受支持实例。"))?;
        let native = updater::native::handles(&installation);
        let key = (
            installation.agent_id,
            request.allow_prerelease,
            installation.source.clone(),
            installation.architecture.clone(),
        );
        if !catalogs.contains_key(&key) {
            let catalog = if native {
                updater::native::catalog(&installation)?
            } else {
                agents::versions::fetch(key.0, key.1)?
            };
            catalogs.insert(key.clone(), catalog);
        }
        let target =
            agents::versions::select(&catalogs[&key], &request.target, request.allow_prerelease)?;
        if installation.version.as_deref() == Some(&target.version) {
            return Err(AppError::new(
                "ALREADY_CURRENT",
                "此实例已是所选版本，无需重复升级。",
            ));
        }
        if request.target == "latest"
            && agents::versions::update_state(installation.version.as_deref(), &target.version)
                != "update_available"
        {
            return Err(AppError::new(
                "NO_NEWER_VERSION",
                "没有可确认的更新版本，请重新检测或指定目标版本。",
            ));
        }
        items.push(storage::model::PlannedItem {
            installation,
            target,
        });
    }
    with_store(state, |store| store.create_plan(items))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppOverview {
    pub app_version: &'static str,
    pub platform: platform::PlatformInfo,
    pub agents: Vec<agents::AgentSupport>,
    pub backup: backup::BackupPolicy,
    pub storage: storage::StorageStatus,
    pub updater: updater::UpdateStatus,
    pub app_release: releases::ReleaseStatus,
    pub validation_mode: bool,
    pub validation_workflow: bool,
}

pub fn overview() -> Result<AppOverview, AppError> {
    Ok(AppOverview {
        app_version: env!("CARGO_PKG_VERSION"),
        platform: platform::current()?,
        agents: agents::catalog(),
        backup: backup::policy(),
        storage: storage::status(),
        updater: updater::status(),
        app_release: releases::status(),
        validation_mode: cfg!(feature = "desktop-validation")
            && std::env::var_os("AGENTUPDATE_VALIDATION_RESULT").is_some(),
        validation_workflow: cfg!(feature = "desktop-validation")
            && std::env::var_os("AGENTUPDATE_VALIDATION_WORKFLOW").is_some(),
    })
}
