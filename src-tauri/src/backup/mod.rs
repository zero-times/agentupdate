pub mod snapshot;
use crate::{
    agents::AgentId,
    error::AppError,
    platform::files,
    storage::{
        model::{Stage, UpdateTransaction},
        Store,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPolicy {
    pub required_before_update: bool,
    pub state: &'static str,
}
pub fn policy() -> BackupPolicy {
    BackupPolicy {
        required_before_update: true,
        state: "available",
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub transaction_id: String,
    pub installation_id: String,
    pub original_version: String,
    pub target_version: String,
    pub config_root: PathBuf,
    pub config_directory_existed: bool,
    #[serde(default = "default_config_mode")]
    pub config_directory_mode: u32,
    pub package: snapshot::Snapshot,
    pub config: snapshot::Snapshot,
    pub launcher_link: PathBuf,
    #[serde(default)]
    pub absent_config_directories: Vec<String>,
}
fn default_config_mode() -> u32 {
    0o700
}
pub fn config_names(agent: AgentId) -> &'static [&'static str] {
    match agent {
        AgentId::Codex => &["config.toml", "auth.json"],
        AgentId::Deepseek => &["cordis.patch.yml", ".credentials.yaml"],
        AgentId::Claude => &[
            ".claude/settings.json",
            ".claude/settings.local.json",
            ".claude/.credentials.json",
            ".claude.json",
        ],
        AgentId::Gemini => &[
            "settings.json",
            "oauth_creds.json",
            "google_accounts.json",
            "mcp-oauth-tokens.json",
        ],
    }
}
pub fn config_root(home: &Path, agent: AgentId) -> Result<PathBuf, AppError> {
    let variable = match agent {
        AgentId::Codex => "CODEX_HOME",
        AgentId::Deepseek => "DSH_HOME",
        AgentId::Claude => "CLAUDE_CONFIG_DIR",
        AgentId::Gemini => "GEMINI_CLI_HOME",
    };
    // Injected homes must never inherit the user's live configuration overrides.
    let real_home = std::env::var_os("HOME").map(PathBuf::from);
    let override_root = real_home
        .as_deref()
        .filter(|real| fs::canonicalize(real).ok() == fs::canonicalize(home).ok())
        .and_then(|_| std::env::var_os(variable));
    if agent == AgentId::Claude {
        if override_root.is_some_and(|p| PathBuf::from(p) != home.join(".claude")) {
            return Err(AppError::new(
                "CONFIG_PATH",
                "Claude 自定义配置目录暂不能安全备份，请使用默认配置目录。",
            ));
        }
        return Ok(home.to_path_buf());
    }
    let root = if let Some(value) = override_root {
        let value = PathBuf::from(value);
        if !value.is_absolute() {
            return Err(AppError::new(
                "CONFIG_PATH",
                "工具自定义配置目录必须为绝对路径。",
            ));
        }
        if agent == AgentId::Gemini {
            value.join(".gemini")
        } else {
            value
        }
    } else {
        home.join(match agent {
            AgentId::Codex => ".codex",
            AgentId::Deepseek => ".dsh",
            AgentId::Claude => ".claude",
            AgentId::Gemini => ".gemini",
        })
    };
    snapshot::ensure_parents(root.parent().unwrap(), Path::new(root.file_name().unwrap()))?;
    Ok(root)
}
pub fn prepare(
    store: &mut Store,
    transaction: &UpdateTransaction,
    home: &Path,
) -> Result<Manifest, AppError> {
    if transaction.stage != Stage::BackingUp {
        return Err(AppError::new(
            "INVALID_TRANSITION",
            "当前任务不能建立备份。",
        ));
    }
    let package = transaction
        .installation
        .package_root
        .as_ref()
        .ok_or_else(|| AppError::new("INSTANCE_UNAVAILABLE", "无法定位原安装。"))?;
    let package_snapshot = snapshot::scan(package, None)?;
    let root = config_root(home, transaction.installation.agent_id)?;
    let existed = fs::symlink_metadata(&root).is_ok();
    use std::os::unix::fs::MetadataExt;
    let config_directory_mode = match fs::symlink_metadata(&root) {
        Ok(m)
            if m.is_dir()
                && !m.file_type().is_symlink()
                && m.uid() == unsafe { libc::geteuid() }
                && m.mode() & 0o700 == 0o700 =>
        {
            m.mode() & 0o777
        }
        Ok(_) => {
            return Err(AppError::new(
                "CONFIG_PERMISSIONS",
                "配置目录不是当前用户可读写的普通目录，停止准备。",
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0o700,
        Err(_) => return Err(AppError::new("CONFIG_PERMISSIONS", "配置目录无法读取。")),
    };
    let config_snapshot = config_snapshot(&root, transaction.installation.agent_id)?;
    if transaction.installation.agent_id == AgentId::Claude
        && config_snapshot.entries.iter().any(|e| {
            !(e.path == ".claude" || e.path == ".claude.json" || e.path.starts_with(".claude/"))
        })
    {
        return Err(AppError::new(
            "EXTERNAL_SYMLINK",
            "Claude 配置链接离开受管范围，停止备份。",
        ));
    }

    if transaction.installation.agent_id != AgentId::Deepseek
        && config_names(transaction.installation.agent_id)
            .iter()
            .any(|name| {
                config_snapshot
                    .entries
                    .iter()
                    .any(|e| e.path == *name && e.kind == "directory")
            })
    {
        return Err(AppError::new(
            "CONFIG_TYPE",
            "受管配置路径是目录，不能安全恢复为配置文件。",
        ));
    }
    if config_snapshot.bytes > 64 * 1024 * 1024 {
        return Err(AppError::new(
            "CONFIG_TOO_LARGE",
            "配置范围超过 64 MiB，请先检查配置目录。",
        ));
    }
    snapshot::require_space(
        snapshot::available_space(&store.root)?,
        (package_snapshot.bytes + config_snapshot.bytes).saturating_mul(2) + 512 * 1024 * 1024,
    )?;
    let launcher_link = fs::read_link(&transaction.installation.command_path)
        .map_err(|_| AppError::new("LAUNCHER_CHANGED", "原 npm 启动链接已变化，停止更新。"))?;
    if fs::canonicalize(&transaction.installation.command_path)
        .ok()
        .as_ref()
        != Some(&transaction.installation.resolved_command)
    {
        return Err(AppError::new(
            "LAUNCHER_CHANGED",
            "原命令路径已变化，停止更新。",
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let partial = store.begin_restore_point(&id)?;
    snapshot::copy(package, &partial.join("package"), &package_snapshot)?;
    snapshot::copy(&root, &partial.join("config"), &config_snapshot)?;
    let manifest = Manifest {
        schema_version: 1,
        id: id.clone(),
        transaction_id: transaction.id.clone(),
        installation_id: transaction.installation.id.clone(),
        original_version: transaction.installation.version.clone().unwrap(),
        target_version: transaction.target.version.clone(),
        config_root: root,
        config_directory_existed: existed,
        config_directory_mode,
        package: package_snapshot,
        config: config_snapshot,
        launcher_link,
        absent_config_directories: if transaction.installation.agent_id == AgentId::Claude
            && fs::symlink_metadata(home.join(".claude")).is_err()
        {
            vec![".claude".into()]
        } else {
            vec![]
        },
    };
    files::write_atomic(
        &partial.join("manifest.json"),
        &serde_json::to_vec(&manifest)
            .map_err(|_| AppError::new("MANIFEST_WRITE", "无法记录恢复清单。"))?,
    )?;
    verify_material(&partial, &manifest)?;
    store.publish_restore_point(&transaction.id, &id)?;
    Ok(manifest)
}
pub fn load(
    store: &Store,
    transaction: &UpdateTransaction,
) -> Result<(PathBuf, Manifest), AppError> {
    let id = transaction
        .restore_point_id
        .as_deref()
        .ok_or_else(|| AppError::new("RESTORE_NOT_READY", "没有可用恢复点。"))?;
    let point = store.verify_restore_point(id)?;
    if point.transaction_id != transaction.id {
        return Err(AppError::new("RESTORE_IDENTITY", "恢复点不属于此任务。"));
    }
    let directory = store.root.join(point.relative_path);
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(directory.join("manifest.json"))
            .map_err(|_| AppError::new("RESTORE_MISSING", "恢复清单无法读取。"))?,
    )
    .map_err(|_| AppError::new("RESTORE_CORRUPT", "恢复清单格式损坏。"))?;
    if transaction.installation.agent_id == AgentId::Claude
        && manifest.config.entries.iter().any(|e| {
            !(e.path == ".claude" || e.path == ".claude.json" || e.path.starts_with(".claude/"))
        })
    {
        return Err(AppError::new(
            "RESTORE_CORRUPT",
            "Claude 恢复配置范围无效。",
        ));
    }
    verify_material(&directory, &manifest)?;
    Ok((directory, manifest))
}
pub fn verify_material(directory: &Path, manifest: &Manifest) -> Result<(), AppError> {
    if manifest
        .absent_config_directories
        .iter()
        .any(|p| p != ".claude")
    {
        return Err(AppError::new("RESTORE_CORRUPT", "恢复目录缺失记录无效。"));
    }
    snapshot::verify(&directory.join("package"), &manifest.package)?;
    snapshot::verify(&directory.join("config"), &manifest.config)
}

/// Harness profiles contain installation-dependent node_modules projections; backup
/// the user's YAML/manifests/locks, while the package snapshot owns runtime dependencies.
pub fn config_snapshot(root: &Path, agent: AgentId) -> Result<snapshot::Snapshot, AppError> {
    let mut names: Vec<String> = config_names(agent).iter().map(|s| s.to_string()).collect();
    if agent == AgentId::Deepseek {
        let profiles = root.join("profiles");
        if let Ok(entries) = fs::read_dir(&profiles) {
            for (index, entry) in entries.enumerate() {
                if index >= 128 {
                    return Err(AppError::new(
                        "CONFIG_TOO_LARGE",
                        "Harness profile 数量超出备份范围。",
                    ));
                }
                let entry = entry
                    .map_err(|_| AppError::new("CONFIG_READ", "Harness profile 目录不可读取。"))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == "node_modules" || name.starts_with('.') {
                    continue;
                }
                snapshot::relative(&name)?;
                let meta = fs::symlink_metadata(entry.path())
                    .map_err(|_| AppError::new("CONFIG_READ", "Harness profile 不可读取。"))?;
                if meta.file_type().is_symlink() {
                    return Err(AppError::new(
                        "EXTERNAL_SYMLINK",
                        "Harness profile 目录不能为外部链接。",
                    ));
                }
                if !meta.is_dir() {
                    continue;
                }
                for leaf in [
                    "cordis.patch.yml",
                    "cordis.yml",
                    "package.json",
                    "pnpm-lock.yaml",
                    "pnpm-workspace.yaml",
                    ".credentials.yaml",
                ] {
                    names.push(format!("profiles/{name}/{leaf}"));
                }
            }
        }
    }
    for name in &names {
        if fs::symlink_metadata(root.join(name)).is_ok_and(|m| m.is_dir()) {
            return Err(AppError::new("CONFIG_TYPE", "受管配置文件不能为目录。"));
        }
    }
    let refs: Vec<_> = names.iter().map(String::as_str).collect();
    snapshot::scan(root, Some(&refs))
}
