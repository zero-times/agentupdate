use crate::{
    backup::{self, snapshot},
    error::AppError,
    platform::{files, locks::OperationLock},
    storage::{model::UpdateTransaction, Store},
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreDetails {
    pub transaction_id: String,
    pub restore_point_id: Option<String>,
    pub verified: bool,
    pub error_code: Option<String>,
    pub bytes: u64,
    pub config_root: Option<PathBuf>,
    pub config_paths: Vec<String>,
    pub can_delete: bool,
    pub protection_reason: Option<String>,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    Diagnostics,
    Restore,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub id: String,
    pub directory: PathBuf,
    pub sensitive: bool,
}
fn protection(store: &Store, tx: &UpdateTransaction) -> Result<Option<String>, AppError> {
    if tx.stage.active() {
        return Ok(Some("进行中或待恢复事务的材料不能清理。".into()));
    }
    let point = match &tx.restore_point_id {
        Some(id) => store.restore_point(id)?,
        None => return Ok(Some("此事务没有恢复点。".into())),
    };
    for other in store.plans()?.into_iter().flat_map(|p| p.transactions) {
        if other.installation.id != tx.installation.id
            || other.id == tx.id
            || other.restore_point_id.is_none()
        {
            continue;
        }
        let other_point = store.restore_point(other.restore_point_id.as_ref().unwrap())?;
        if (other_point.created_at, &other_point.id) > (point.created_at, &point.id)
            && backup::load(store, &other).is_ok()
        {
            return Ok(None);
        }
    }
    Ok(Some("至少保留此实例最近一个完整可用的恢复点。".into()))
}
pub fn details(store: &Store, id: &str) -> Result<RestoreDetails, AppError> {
    let tx = store.transaction(id)?;
    let material = backup::load(store, &tx);
    let reason = protection(store, &tx)?;
    match material {
        Ok((_, m)) => Ok(RestoreDetails {
            transaction_id: id.into(),
            restore_point_id: tx.restore_point_id,
            verified: true,
            error_code: None,
            bytes: m.package.bytes + m.config.bytes,
            config_root: Some(m.config_root),
            config_paths: m
                .config
                .entries
                .into_iter()
                .map(|e| format!("{} · {}", e.path, e.kind))
                .collect(),
            can_delete: reason.is_none(),
            protection_reason: reason,
        }),
        Err(e) => Ok(RestoreDetails {
            transaction_id: id.into(),
            restore_point_id: tx.restore_point_id,
            verified: false,
            error_code: Some(e.code.into()),
            bytes: 0,
            config_root: None,
            config_paths: vec![],
            can_delete: false,
            protection_reason: Some("材料未通过校验，保留供人工处理。".into()),
        }),
    }
}
fn version(value: Option<&str>) -> Option<String> {
    value
        .and_then(|s| semver::Version::parse(s).ok())
        .map(|v| v.to_string())
}
fn code(value: &str) -> String {
    if [
        "AGENTUPDATE_TEST_DATA_DIR",
        "AGENTUPDATE_VALIDATION_RESULT",
        "AGENT_RUNNING",
        "AGENT_VERSION",
        "ARTIFACT_INTEGRITY",
        "ARTIFACT_READ",
        "ARTIFACT_SIZE",
        "ARTIFACT_SOURCE",
        "BACKUP_CHANGED",
        "BACKUP_PERMISSIONS",
        "BACKUP_READ",
        "BACKUP_WRITE",
        "BROKEN_SYMLINK",
        "CANCELLED",
        "CANCEL_FAILED",
        "CARGO_PKG_VERSION",
        "CLEANUP_CONFLICT",
        "CLEANUP_FAILED",
        "CLEANUP_PENDING",
        "COMMAND_UNAVAILABLE",
        "CONFIG_CHANGED",
        "CONFIG_CONFIRMATION_REQUIRED",
        "CONFIG_PATH",
        "CONFIG_PERMISSIONS",
        "CONFIG_RESTORE_WRITE",
        "CONFIG_ROOT_CHANGED",
        "CONFIG_TOO_LARGE",
        "CONFIG_TYPE",
        "CONFIG_TYPE_CHANGED",
        "CONFIG_VERIFY",
        "DATABASE_CORRUPT",
        "DATABASE_NEWER",
        "DATABASE_OPEN",
        "DATABASE_OPERATION",
        "DATABASE_RECORD",
        "DATABASE_RECORD_INVALID",
        "DATA_DIRECTORY",
        "DATA_PATH",
        "DATA_PERMISSIONS",
        "DATA_SYNC",
        "DATA_WRITE",
        "DISCOVERY_FAILED",
        "DISK_FULL",
        "DISK_SPACE",
        "DOWNLOAD_FAILED",
        "DOWNLOAD_HTTP",
        "DOWNLOAD_INTERRUPTED",
        "DOWNLOAD_SETUP",
        "EXPORT_FAILED",
        "EXPORT_NOT_FOUND",
        "EXPORT_OPEN",
        "EXPORT_VERIFY",
        "EXTERNAL_SYMLINK",
        "FILE_UNAVAILABLE",
        "HEALTH_FAILED",
        "HISTORY_FAILED",
        "HOME_UNAVAILABLE",
        "INSTALLATION_CHANGED",
        "INSTALL_SWAP",
        "INSTANCE_BUSY",
        "INSTANCE_CHANGED",
        "INSTANCE_UNAVAILABLE",
        "INTERRUPTED_BEFORE_INSTALL",
        "INTERRUPTED_MUTATION",
        "INVALID_DISCOVERY_PATH",
        "INVALID_ID",
        "INVALID_PLAN",
        "INVALID_PLAN_STATE",
        "INVALID_TRANSITION",
        "INVALID_VERSION",
        "LAUNCHER_CHANGED",
        "LAUNCHER_PATH",
        "LAUNCHER_WRITE",
        "LOCK_UNAVAILABLE",
        "LOCK_UNSAFE",
        "MANIFEST_WRITE",
        "NETWORK_SETUP",
        "NODE_UNAVAILABLE",
        "NODE_VERSION",
        "NO_COLOR",
        "NPM_UNAVAILABLE",
        "OPERATION_BUSY",
        "PLAN_FAILED",
        "PLAN_INTERRUPTED",
        "PLAN_NOT_FOUND",
        "PLAN_PAUSED",
        "PRERELEASE_OPT_IN",
        "PROBE_DIRECTORY",
        "PROCESS_FAILED",
        "PROCESS_OUTPUT",
        "PROCESS_OUTPUT_LIMIT",
        "PROCESS_START",
        "PROCESS_TIMEOUT",
        "PROCESS_WAIT",
        "QUEUE_INTERRUPTED",
        "RECOVERY_CHECK_FAILED",
        "REDACTED",
        "RESCUE_WRITE",
        "RESTORE_CORRUPT",
        "RESTORE_DIRECTORY",
        "RESTORE_EXISTS",
        "RESTORE_IDENTITY",
        "RESTORE_MISSING",
        "RESTORE_NOT_FOUND",
        "RESTORE_NOT_READY",
        "RESTORE_PATH",
        "RESTORE_PERMISSION",
        "RESTORE_PERMISSIONS",
        "RESTORE_PROTECTED",
        "RESTORE_PUBLISH",
        "RESTORE_SWAP",
        "RESTORE_VERIFY",
        "RESTORE_VERSION",
        "RESTORE_WRITE",
        "ROLLBACK_CHECK_FAILED",
        "ROLLBACK_INTERRUPTED",
        "ROLLBACK_NOT_AVAILABLE",
        "SENSITIVE_EXPORT_CONFIRMATION",
        "SNAPSHOT_CORRUPT",
        "SNAPSHOT_LIMIT",
        "SNAPSHOT_PATH_ENCODING",
        "SNAPSHOT_SOURCE",
        "STAGED_INSTALL_FAILED",
        "STAGED_VERSION_FAILED",
        "STAGE_SETUP",
        "STATE_UNAVAILABLE",
        "SWAP_CONFLICT",
        "TMPDIR",
        "TRANSACTION_CONFLICT",
        "TRANSACTION_NOT_FOUND",
        "UNSAFE_DATA_PATH",
        "UNSAFE_SNAPSHOT_PATH",
        "UNSUPPORTED_ARCH",
        "UNSUPPORTED_FILE_TYPE",
        "UNSUPPORTED_INSTALLATION",
        "UNSUPPORTED_PLATFORM",
        "UPDATE_INTERRUPTED",
        "VERIFY_PATH",
        "VERIFY_VERSION",
        "VERSION_HTTP",
        "VERSION_NETWORK",
        "VERSION_NOT_FOUND",
        "VERSION_QUERY_FAILED",
        "VERSION_RATE_LIMIT",
        "VERSION_RESPONSE",
        "queued",
        "preflight",
        "backing_up",
        "prepared",
        "installing",
        "verifying",
        "succeeded",
        "preparation_failed",
        "needs_recovery",
        "recovering",
        "restored",
        "recovery_failed",
        "cancelled",
        "restore_deleted",
    ]
    .contains(&value)
    {
        value.into()
    } else {
        "REDACTED".into()
    }
}
pub fn diagnostic(store: &Store, id: &str) -> Result<serde_json::Value, AppError> {
    let tx = store.transaction(id)?;
    let events: Vec<_> = store
        .events(id)?
        .into_iter()
        .map(|e| serde_json::json!({"code":code(&e.code),"createdAt":e.created_at}))
        .collect();
    Ok(
        serde_json::json!({"schemaVersion":1,"appVersion":env!("CARGO_PKG_VERSION"),"platform":"macos","transactionId":tx.id,"planId":tx.plan_id,"tool":tx.installation.agent_id,"originalVersion":version(tx.installation.version.as_deref()),"targetVersion":version(Some(&tx.target.version)),"stage":tx.stage,"restorePointId":tx.restore_point_id,"failureCode":tx.failure_code.as_deref().map(code),"createdAt":tx.created_at,"updatedAt":tx.updated_at,"events":events}),
    )
}
pub fn export(
    store: &Store,
    id: &str,
    kind: ExportKind,
    confirmed_sensitive: bool,
) -> Result<ExportResult, AppError> {
    let _lock = OperationLock::acquire(&store.root.join("locks/operation.lock"))?;
    let sensitive = matches!(kind, ExportKind::Restore);
    if sensitive && !confirmed_sensitive {
        return Err(AppError::new(
            "SENSITIVE_EXPORT_CONFIRMATION",
            "完整恢复点可能包含登录令牌，需要明确确认导出。",
        ));
    }
    let export_root = store.root.join("exports");
    files::private_directory(&export_root)?;
    let export_id = uuid::Uuid::new_v4().to_string();
    let partial = export_root.join(format!("{export_id}.partial"));
    let destination = export_root.join(&export_id);
    files::private_directory(&partial)?;
    let result = (|| {
        match kind {
            ExportKind::Diagnostics => files::write_atomic(
                &partial.join("diagnostics.json"),
                &serde_json::to_vec_pretty(&diagnostic(store, id)?)
                    .map_err(|_| AppError::new("EXPORT_FAILED", "日志导出未完成。"))?,
            )?,
            ExportKind::Restore => {
                let tx = store.transaction(id)?;
                let (directory, _) = backup::load(store, &tx)?;
                let tree = snapshot::scan(&directory, None)?;
                snapshot::require_space(
                    snapshot::available_space(&export_root)?,
                    tree.bytes.saturating_mul(2) + 256 * 1024 * 1024,
                )?;
                snapshot::copy(&directory, &partial.join("archive"), &tree)?;
                snapshot::materialize(&partial.join("archive"), &tree, &partial.join("restore"))?;
                fs::remove_dir_all(partial.join("archive"))
                    .map_err(|_| AppError::new("EXPORT_FAILED", "导出暂存清理失败。"))?;
                if snapshot::scan(&partial.join("restore"), None)?.entries != tree.entries {
                    return Err(AppError::new("EXPORT_VERIFY", "导出的恢复材料未通过校验。"));
                }
                files::write_atomic(&partial.join("README.txt"),"此导出含完整旧版本、原配置与变化副本，可能含凭据。请私下保管。对照 restore/manifest.json 按 AgentUpdate 人工恢复说明核验每个文件；不要上传原始配置。\n".as_bytes())?;
            }
        }
        files::sync_directory(&partial)?;
        fs::rename(&partial, &destination)
            .map_err(|_| AppError::new("EXPORT_FAILED", "导出目录发布失败。"))?;
        files::sync_directory(&export_root)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&partial);
    }
    result?;
    Ok(ExportResult {
        id: export_id,
        directory: destination,
        sensitive,
    })
}
pub fn reveal_export(store: &Store, id: &str) -> Result<(), AppError> {
    super::validate_id(id)?;
    let directory = store.root.join("exports").join(id);
    if fs::canonicalize(&directory).ok().as_deref() != Some(directory.as_path()) {
        return Err(AppError::new(
            "EXPORT_NOT_FOUND",
            "导出目录不存在或已改变。",
        ));
    }
    let status = std::process::Command::new("/usr/bin/open")
        .arg(&directory)
        .status()
        .map_err(|_| AppError::new("EXPORT_OPEN", "无法打开导出目录。"))?;
    if !status.success() {
        return Err(AppError::new("EXPORT_OPEN", "无法打开导出目录。"));
    }
    Ok(())
}
pub fn cleanup(store: &mut Store, id: &str) -> Result<(), AppError> {
    let _lock = OperationLock::acquire(&store.root.join("locks/operation.lock"))?;
    let tx = store.transaction(id)?;
    if let Some(reason) = protection(store, &tx)? {
        return Err(AppError::new("RESTORE_PROTECTED", reason));
    }
    let (directory, _) = backup::load(store, &tx)?;
    let trash = store.root.join("trash");
    files::private_directory(&trash)?;
    let target = trash.join(tx.restore_point_id.as_ref().unwrap());
    if fs::symlink_metadata(&target).is_ok() {
        return Err(AppError::new(
            "CLEANUP_CONFLICT",
            "存在待处理清理材料，未覆盖。",
        ));
    }
    // Commit reference removal first: a crash can leave extra private files, never a
    // live database reference to a directory already moved away.
    store.detach_restore(&tx)?;
    fs::rename(&directory, &target).map_err(|_| {
        AppError::new(
            "CLEANUP_PENDING",
            "引用已解除，原恢复目录尚未移动，请人工清理已解除引用的材料。",
        )
    })?;
    files::sync_directory(directory.parent().unwrap())?;
    files::sync_directory(&trash)?;
    fs::remove_dir_all(&target).map_err(|_| {
        AppError::new(
            "CLEANUP_PENDING",
            "引用已解除，私有 trash 目录尚未删除，请保留记录并人工清理。",
        )
    })?;
    files::sync_directory(&trash)
}
