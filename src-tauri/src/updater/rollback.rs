use crate::{
    backup::{self, snapshot},
    error::AppError,
    platform::files,
    storage::{
        model::{Stage, UpdateTransaction},
        Store,
    },
    updater::npm,
};
use serde::Serialize;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackPreview {
    pub transaction_id: String,
    pub original_version: String,
    pub configuration_changed: bool,
    pub restores_absent_files: bool,
    pub preservation_location: String,
}
fn error(code: &'static str, message: &str) -> AppError {
    AppError::new(code, message)
}
fn selected_paths(manifest: &backup::Manifest, tx: &UpdateTransaction) -> Vec<String> {
    let mut paths: Vec<String> = backup::config_names(tx.installation.agent_id)
        .iter()
        .map(|s| s.to_string())
        .collect();
    paths.extend(
        manifest
            .config
            .entries
            .iter()
            .filter(|e| e.kind != "directory")
            .map(|e| e.path.clone()),
    );
    paths.sort();
    paths.dedup();
    paths
}
fn config_directory_state(root: &Path) -> Result<(bool, u32), AppError> {
    match fs::symlink_metadata(root) {
        Ok(m)
            if m.is_dir()
                && !m.file_type().is_symlink()
                && m.uid() == unsafe { libc::geteuid() }
                && m.mode() & 0o700 == 0o700 =>
        {
            Ok((true, m.mode() & 0o777))
        }
        Ok(_) => Err(error(
            "CONFIG_RESTORE_PERMISSION",
            "当前配置目录类型、所有权或权限不符合恢复条件。",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((false, 0)),
        Err(_) => Err(error("CONFIG_RESTORE_PERMISSION", "当前配置目录无法核验。")),
    }
}
fn scan_current(
    manifest: &backup::Manifest,
    tx: &UpdateTransaction,
) -> Result<snapshot::Snapshot, AppError> {
    let selected = selected_paths(manifest, tx);
    let refs: Vec<_> = selected.iter().map(String::as_str).collect();
    snapshot::scan(&manifest.config_root, Some(&refs))
}
fn check_paths(tx: &UpdateTransaction) -> Result<(), AppError> {
    let i = &tx.installation;
    if super::native::handles(i) {
        return super::native::validate_paths(i);
    }
    let prefix = i
        .prefix
        .as_ref()
        .ok_or_else(|| error("RESTORE_PATH", "恢复实例没有标准安装目录。"))?;
    let package = i
        .package_root
        .as_ref()
        .ok_or_else(|| error("RESTORE_PATH", "恢复实例没有标准包目录。"))?;
    let command_parent = i
        .command_path
        .parent()
        .ok_or_else(|| error("RESTORE_PATH", "恢复命令没有父目录。"))?;
    if *package != prefix.join("lib/node_modules").join(i.agent_id.package())
        || i.command_path.file_name() != Some(std::ffi::OsStr::new(i.agent_id.key()))
        || fs::canonicalize(command_parent).ok().as_deref() != Some(prefix.join("bin").as_path())
    {
        return Err(error("RESTORE_PATH", "恢复范围与标准 npm 实例不一致。"));
    }
    for path in [
        prefix.as_path(),
        package.parent().unwrap(),
        prefix.join("bin").as_path(),
    ] {
        if fs::canonicalize(path).ok().as_deref() != Some(path) {
            return Err(error("RESTORE_PATH", "恢复路径的目录存在链接或已变化。"));
        }
        let m =
            fs::symlink_metadata(path).map_err(|_| error("RESTORE_PATH", "恢复目录无法读取。"))?;
        if !m.is_dir()
            || m.uid() != unsafe { libc::geteuid() }
            || m.permissions().mode() & 0o200 == 0
        {
            return Err(error(
                "RESTORE_PERMISSION",
                "恢复目录不是当前用户可写的普通目录。",
            ));
        }
    }
    if let Ok(m) = fs::symlink_metadata(package) {
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err(error("RESTORE_PATH", "当前安装目录的类型已变化。"));
        }
    }
    if let Ok(m) = fs::symlink_metadata(&i.command_path) {
        if !m.file_type().is_symlink() {
            return Err(error(
                "RESTORE_PATH",
                "当前启动命令已成为普通文件，停止自动覆盖。",
            ));
        }
    }
    Ok(())
}
pub fn preview(store: &Store, id: &str, home: &Path) -> Result<RollbackPreview, AppError> {
    let tx = store.transaction(id)?;
    if !matches!(
        tx.stage,
        Stage::Succeeded | Stage::NeedsRecovery | Stage::RecoveryFailed
    ) {
        return Err(error("ROLLBACK_NOT_AVAILABLE", "此任务当前不能回滚。"));
    }
    let (_, manifest) = backup::load(store, &tx)?;
    check_paths(&tx)?;
    if backup::config_root(home, tx.installation.agent_id)? != manifest.config_root {
        return Err(error(
            "CONFIG_ROOT_CHANGED",
            "配置目录设置与恢复点不同，请先恢复原目录设置。",
        ));
    }
    let current = scan_current(&manifest, &tx)?;
    let state = config_directory_state(&manifest.config_root)?;
    Ok(RollbackPreview {
        transaction_id: id.into(),
        original_version: manifest.original_version,
        configuration_changed: current.entries != manifest.config.entries
            || state.0 != manifest.config_directory_existed
            || (state.0 && state.1 != manifest.config_directory_mode),
        restores_absent_files: manifest.config.entries.iter().any(|e| e.kind == "missing"),
        preservation_location: format!("restores/{}/rescue", manifest.id),
    })
}
fn remove_leaf(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() || m.file_type().is_symlink() => fs::remove_file(path)
            .map_err(|_| error("CONFIG_RESTORE_WRITE", "无法恢复受管配置文件。")),
        Ok(_) => Err(error(
            "CONFIG_TYPE_CHANGED",
            "受管配置已成为目录或特殊文件，请手动检查；未删除。",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(error("CONFIG_RESTORE_WRITE", "无法读取受管配置路径。")),
    }
}
pub fn restore_config(
    directory: &Path,
    manifest: &backup::Manifest,
    staged: &Path,
) -> Result<(), AppError> {
    backup::verify_material(directory, manifest)?;
    if fs::symlink_metadata(&manifest.config_root).is_err() {
        fs::create_dir_all(&manifest.config_root)
            .map_err(|_| error("CONFIG_RESTORE_WRITE", "无法创建原配置目录。"))?;
        fs::set_permissions(&manifest.config_root, fs::Permissions::from_mode(0o700))
            .map_err(|_| error("CONFIG_RESTORE_WRITE", "无法限制配置目录权限。"))?;
    }
    snapshot::ensure_parents(&manifest.config_root, Path::new("."))?;
    // Check every managed leaf before any deletion or replacement.
    for entry in &manifest.config.entries {
        if entry.kind == "directory" {
            continue;
        }
        let relative = snapshot::relative(&entry.path)?;
        snapshot::ensure_parents(&manifest.config_root, &relative)?;
        let dest = manifest.config_root.join(relative);
        if let Ok(m) = fs::symlink_metadata(dest) {
            if !m.is_file() && !m.file_type().is_symlink() {
                return Err(error(
                    "CONFIG_TYPE_CHANGED",
                    "受管配置类型已改变，请人工处理。",
                ));
            }
        }
    }
    for entry in &manifest.config.entries {
        let relative = snapshot::relative(&entry.path)?;
        let dest = manifest.config_root.join(&relative);
        if entry.kind == "directory" {
            fs::create_dir_all(&dest)
                .map_err(|_| error("CONFIG_RESTORE_WRITE", "无法恢复配置目录。"))?;
            continue;
        }
        if entry.kind == "missing" {
            remove_leaf(&dest)?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|_| error("CONFIG_RESTORE_WRITE", "无法恢复配置父目录。"))?;
        }
        fs::rename(staged.join(&relative), &dest)
            .map_err(|_| error("CONFIG_RESTORE_WRITE", "配置原子替换失败；保留恢复材料。"))?;
        files::sync_directory(dest.parent().unwrap())?;
    }
    for entry in manifest
        .config
        .entries
        .iter()
        .rev()
        .filter(|e| e.kind == "directory")
    {
        let dest = manifest.config_root.join(snapshot::relative(&entry.path)?);
        fs::set_permissions(&dest, fs::Permissions::from_mode(entry.mode & 0o777))
            .map_err(|_| error("CONFIG_RESTORE_WRITE", "无法恢复配置父目录权限。"))?;
        files::sync_directory(&dest)?;
    }
    for path in &manifest.absent_config_directories {
        let dest = manifest.config_root.join(snapshot::relative(path)?);
        if dest.is_dir()
            && fs::read_dir(&dest)
                .map_err(|_| error("CONFIG_VERIFY", "无法核验原配置父目录。"))?
                .next()
                .is_none()
        {
            fs::remove_dir(&dest)
                .map_err(|_| error("CONFIG_RESTORE_WRITE", "无法恢复原配置父目录缺失状态。"))?;
        }
    }
    if !manifest.config_directory_existed {
        if fs::read_dir(&manifest.config_root)
            .map_err(|_| error("CONFIG_VERIFY", "无法验证原始缺失状态。"))?
            .next()
            .is_none()
        {
            fs::remove_dir(&manifest.config_root)
                .map_err(|_| error("CONFIG_RESTORE_WRITE", "无法恢复原始目录缺失状态。"))?;
            files::sync_directory(manifest.config_root.parent().unwrap())?;
        }
    } else {
        fs::set_permissions(
            &manifest.config_root,
            fs::Permissions::from_mode(manifest.config_directory_mode & 0o777),
        )
        .map_err(|_| error("CONFIG_RESTORE_WRITE", "无法恢复原配置目录权限。"))?;
        files::sync_directory(&manifest.config_root)?;
    }
    Ok(())
}
pub fn execute(
    store: &mut Store,
    id: &str,
    context: &npm::Context,
    confirm_changed_config: bool,
) -> Result<UpdateTransaction, AppError> {
    let preview = preview(store, id, &context.home)?;
    let original = store.transaction(id)?;
    let _lock =
        crate::platform::locks::OperationLock::npm(original.installation.prefix.as_ref().unwrap())?;
    npm::check_running(&original.installation)?;
    if preview.configuration_changed && !confirm_changed_config {
        return Err(error(
            "CONFIG_CONFIRMATION_REQUIRED",
            "配置在更新后已变化。确认后会先保存变化副本，再恢复升级前配置。",
        ));
    }
    let (directory, manifest) = backup::load(store, &original)?;
    let current = scan_current(&manifest, &original)?;
    let current_directory = config_directory_state(&manifest.config_root)?;
    snapshot::require_space(
        snapshot::available_space(original.installation.prefix.as_ref().unwrap())?,
        manifest.package.bytes.saturating_mul(2) + current.bytes + 64 * 1024 * 1024,
    )?;
    let attempt = uuid::Uuid::new_v4().to_string();
    let work = store.root.join("recovery-work").join(&attempt);
    files::private_directory(&work)?;
    let package = original.installation.package_root.as_ref().unwrap();
    let replacement = package
        .parent()
        .unwrap()
        .join(format!(".agentupdate-recovery-{attempt}"));
    snapshot::materialize(&directory.join("package"), &manifest.package, &replacement)?;
    let relative = original
        .installation
        .resolved_command
        .strip_prefix(package)
        .map_err(|_| error("RESTORE_PATH", "原命令不属于原包目录。"))?;
    let (version, arch) = super::native::verify(
        &original.installation,
        &replacement.join(relative),
        &context.home,
        true,
    )?;
    if version != manifest.original_version || arch != original.installation.architecture {
        return Err(error("RESTORE_VERSION", "离线重建的旧版本未通过验证。"));
    }
    let staged_config = work.join("config");
    snapshot::materialize(&directory.join("config"), &manifest.config, &staged_config)?;
    let rescue = directory.join("rescue").join(&attempt);
    files::private_directory(&rescue)?;
    snapshot::copy(&manifest.config_root, &rescue.join("config"), &current)?;
    files::write_atomic(
        &rescue.join("manifest.json"),
        &serde_json::to_vec(&serde_json::json!({"config":current,"directoryExisted":current_directory.0,"directoryMode":current_directory.1}))
            .map_err(|_| error("RESCUE_WRITE", "无法保存更新后配置清单。"))?,
    )?;
    // Refuse changes made while preparing recovery rather than overwriting unsaved edits.
    if scan_current(&manifest, &original)?.entries != current.entries
        || config_directory_state(&manifest.config_root)? != current_directory
    {
        return Err(error(
            "CONFIG_CHANGED",
            "准备恢复期间配置又发生变化，请重试。",
        ));
    }
    npm::check_running(&original.installation)?;
    check_paths(&original)?;
    let tx = store.transition(id, original.stage, Stage::Recovering, None)?;
    let result = (|| {
        if package.exists() {
            let displaced = package
                .parent()
                .unwrap()
                .join(format!(".agentupdate-displaced-{attempt}"));
            fs::rename(package, &displaced)
                .map_err(|_| error("RESTORE_SWAP", "无法保留当前安装；恢复未确认完成。"))?;
            files::sync_directory(package.parent().unwrap())?;
        }
        fs::rename(&replacement, package)
            .map_err(|_| error("RESTORE_SWAP", "旧安装发布失败；保留所有恢复材料。"))?;
        files::sync_directory(package.parent().unwrap())?;
        npm::replace_link(&tx.installation.command_path, &manifest.launcher_link)?;
        #[cfg(feature = "desktop-validation")]
        if std::env::var("AGENTUPDATE_TEST_FAIL_RESTORE_AGENT")
            .ok()
            .as_deref()
            == Some(tx.installation.agent_id.key())
        {
            return Err(error(
                "INJECTED_RESTORE_FAILURE",
                "隔离测试：模拟配置恢复前失败。",
            ));
        }
        restore_config(&directory, &manifest, &staged_config)?;
        let resolved = fs::canonicalize(&tx.installation.command_path)
            .map_err(|_| error("RESTORE_VERIFY", "恢复后命令无法解析。"))?;
        if resolved != tx.installation.resolved_command {
            return Err(error("RESTORE_VERIFY", "恢复后的实际命令路径不一致。"));
        }
        let (version, arch) =
            super::native::verify(&tx.installation, &resolved, &context.home, true)?;
        if version != manifest.original_version || arch != tx.installation.architecture {
            return Err(error("RESTORE_VERIFY", "恢复后的实际版本或架构不一致。"));
        }
        let selected: Vec<_> = manifest
            .config
            .entries
            .iter()
            .filter(|e| e.kind != "directory")
            .map(|e| e.path.as_str())
            .collect();
        let mut restored = snapshot::scan(&manifest.config_root, Some(&selected))?;
        restored.entries.retain(|entry| {
            !(entry.kind == "directory" && manifest.absent_config_directories.contains(&entry.path))
        });
        if restored.entries != manifest.config.entries {
            return Err(error(
                "CONFIG_VERIFY",
                "恢复后配置内容、权限或缺失状态不一致。",
            ));
        }
        Ok(())
    })();
    match result {
        Ok(()) => store.transition(id, Stage::Recovering, Stage::Restored, None),
        Err(error) => {
            store.transition(
                id,
                Stage::Recovering,
                Stage::RecoveryFailed,
                Some(error.code),
            )?;
            Err(error)
        }
    }
}
