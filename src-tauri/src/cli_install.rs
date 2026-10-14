//! First installation into the current user's standard npm prefix. Existing instances
//! are updated through the normal version/configuration backup transaction engine.
use crate::{
    agents::{
        discovery::{self, DiscoveryContext, Installation},
        versions, AgentId,
    },
    backup::{self, snapshot},
    error::AppError,
    platform::{files, locks::OperationLock},
    storage::{
        self,
        model::{Stage, UpdateTransaction},
    },
    updater::npm,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub agent: AgentId,
    pub version: String,
    pub state: String,
    pub command: PathBuf,
    pub package: PathBuf,
    pub installed: snapshot::Snapshot,
    pub failure_code: Option<String>,
    pub created_at: u64,
}
fn err(code: &'static str) -> AppError {
    AppError::new(
        code,
        "安装未确认完成，请查看安装记录；已有工具与配置已保留。",
    )
}
pub fn directory(path: &Path) -> Result<(), AppError> {
    if let Ok(m) = fs::symlink_metadata(path) {
        if !m.is_dir()
            || m.file_type().is_symlink()
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o200 == 0
        {
            return Err(err("INSTALL_PATH"));
        }
    } else {
        fs::create_dir(path).map_err(|_| err("INSTALL_PATH"))?;
    }
    if fs::canonicalize(path).ok().as_deref() != Some(path) {
        return Err(err("INSTALL_PATH"));
    }
    Ok(())
}
fn record_path(root: &Path, id: &str) -> Result<PathBuf, AppError> {
    uuid::Uuid::parse_str(id).map_err(|_| err("INSTALL_ID"))?;
    Ok(root.join("cli-installs").join(id))
}
fn save(work: &Path, record: &Record) -> Result<(), AppError> {
    files::write_atomic(
        &work.join("record.json"),
        &serde_json::to_vec(record).map_err(|_| err("INSTALL_RECORD"))?,
    )
}
pub fn history(root: &Path) -> Result<Vec<Record>, AppError> {
    let mut out = vec![];
    if let Ok(entries) = fs::read_dir(root.join("cli-installs")) {
        for entry in entries.flatten().take(256) {
            let path = entry.path().join("record.json");
            if !path.exists() {
                continue;
            }
            files::regular_private_file(&path)?;
            let record =
                serde_json::from_slice(&fs::read(path).map_err(|_| err("INSTALL_RECORD"))?)
                    .map_err(|_| err("INSTALL_RECORD"))?;
            out.push(record);
        }
    }
    out.sort_by_key(|r: &Record| std::cmp::Reverse(r.created_at));
    Ok(out)
}
pub fn install(
    root: &Path,
    ctx: &DiscoveryContext,
    agent: AgentId,
    allow_prerelease: bool,
) -> Result<Record, AppError> {
    let _lock = OperationLock::acquire(&root.join("locks/operation.lock"))?;
    if discovery::discover(ctx, &[])?
        .instances
        .iter()
        .any(|i| i.agent_id == agent)
    {
        return Err(AppError::new(
            "ALREADY_INSTALLED",
            "已经发现该工具，请升级现有实例以保留安装来源。",
        ));
    }
    let node = ctx
        .directories
        .iter()
        .map(|d| d.join("node"))
        .find(|p| p.is_file())
        .and_then(|p| fs::canonicalize(p).ok())
        .ok_or_else(|| {
            AppError::new(
                "NODE_UNAVAILABLE",
                "安装此 CLI 需要 Node.js 22.12 或更高版本。",
            )
        })?;
    let catalog = versions::fetch(agent, allow_prerelease)?;
    let target = versions::select(&catalog, "latest", allow_prerelease)?;
    install_target(root, ctx, agent, target, node, None)
}
// Separate entry permits immutable official artifacts/cache in isolated integration tests.
pub fn install_target(
    root: &Path,
    ctx: &DiscoveryContext,
    agent: AgentId,
    target: versions::AvailableVersion,
    node: PathBuf,
    cache: Option<PathBuf>,
) -> Result<Record, AppError> {
    if target.prerelease && agent != AgentId::Deepseek {
        return Err(err("PRERELEASE_OPT_IN"));
    }
    let prefix = ctx.home.join(".local");
    directory(&prefix)?;
    directory(&prefix.join("bin"))?;
    directory(&prefix.join("lib"))?;
    directory(&prefix.join("lib/node_modules"))?;
    let package = prefix.join("lib/node_modules").join(agent.package());
    directory(package.parent().unwrap())?;
    let command = prefix.join("bin").join(agent.key());
    let _prefix_lock = OperationLock::npm(&prefix)?;
    if fs::symlink_metadata(&package).is_ok() || fs::symlink_metadata(&command).is_ok() {
        return Err(err("INSTALL_CONFLICT"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    files::private_directory(&root.join("cli-installs"))?;
    let work = record_path(root, &id)?;
    files::private_directory(&work)?;
    let config_root = backup::config_root(&ctx.home, agent)?;
    let config = backup::config_snapshot(&config_root, agent)?;
    if config.bytes > 64 * 1024 * 1024 {
        return Err(err("CONFIG_TOO_LARGE"));
    }
    snapshot::copy(&config_root, &work.join("config"), &config)?;
    files::write_atomic(
        &work.join("config-manifest.json"),
        &serde_json::to_vec(&config).map_err(|_| err("INSTALL_RECORD"))?,
    )?;
    let i = Installation {
        id: id.clone(),
        agent_id: agent,
        command_path: command.clone(),
        resolved_command: package.join("placeholder"),
        package_root: Some(package.clone()),
        prefix: Some(prefix),
        node_path: Some(node),
        version: None,
        architecture: String::new(),
        source: "user_npm".into(),
        supported: true,
        version_verified: false,
        reason: None,
    };
    let ctx_npm = npm::Context {
        home: ctx.home.clone(),
        cache,
        ..npm::Context::current(&i)?
    };
    let tx = UpdateTransaction {
        id: id.clone(),
        plan_id: id.clone(),
        ordinal: 0,
        installation: i,
        target: target.clone(),
        stage: Stage::Prepared,
        restore_point_id: None,
        failure_code: None,
        created_at: storage::now(),
        updated_at: storage::now(),
    };
    let staged = npm::stage(&tx, &ctx_npm, &work.join("stage"))?;
    let tree = snapshot::scan(staged.package_root.as_ref().unwrap(), None)?;
    let mut record = Record {
        id,
        agent,
        version: target.version,
        state: "prepared".into(),
        command,
        package,
        installed: tree.clone(),
        failure_code: None,
        created_at: storage::now(),
    };
    save(&work, &record)?;
    let result = (|| {
        if fs::symlink_metadata(&record.package).is_ok()
            || fs::symlink_metadata(&record.command).is_ok()
        {
            return Err(err("INSTALL_CONFLICT"));
        }
        let archive = work.join("package");
        snapshot::copy(staged.package_root.as_ref().unwrap(), &archive, &tree)?;
        let replacement = record
            .package
            .parent()
            .unwrap()
            .join(format!(".agentupdate-new-{}", record.id));
        snapshot::materialize(&archive, &tree, &replacement)?;
        if backup::config_snapshot(&config_root, agent)?.entries != config.entries {
            return Err(err("CONFIG_CHANGED"));
        }
        record.state = "installing".into();
        save(&work, &record)?;
        fs::rename(&replacement, &record.package).map_err(|_| err("INSTALL_PUBLISH"))?;
        let relative = staged
            .resolved_command
            .strip_prefix(staged.package_root.as_ref().unwrap())
            .map_err(|_| err("INSTALL_LAUNCHER"))?;
        std::os::unix::fs::symlink(record.package.join(relative), &record.command)
            .map_err(|_| err("INSTALL_LAUNCHER"))?;
        let report = discovery::discover(
            &DiscoveryContext {
                home: ctx.home.clone(),
                directories: vec![
                    record.command.parent().unwrap().into(),
                    staged.node_path.as_ref().unwrap().parent().unwrap().into(),
                ],
            },
            &[],
        )?;
        if !report.instances.iter().any(|i| {
            i.agent_id == agent && i.version.as_deref() == Some(&record.version) && i.supported
        }) {
            return Err(err("INSTALL_VERIFY"));
        }
        record.state = "succeeded".into();
        save(&work, &record)
    })();
    if let Err(e) = result {
        record.state = "needs_recovery".into();
        record.failure_code = Some(e.code.into());
        save(&work, &record)?;
        return Err(e);
    }
    Ok(record)
}
pub fn rollback(root: &Path, home: &Path, id: &str) -> Result<Record, AppError> {
    let _lock = OperationLock::acquire(&root.join("locks/operation.lock"))?;
    let work = record_path(root, id)?;
    let path = work.join("record.json");
    files::regular_private_file(&path)?;
    let mut r: Record = serde_json::from_slice(&fs::read(path).map_err(|_| err("INSTALL_RECORD"))?)
        .map_err(|_| err("INSTALL_RECORD"))?;
    let prefix = home.join(".local");
    directory(&prefix)?;
    let _prefix = OperationLock::npm(&prefix)?;
    if r.id != id
        || r.package != prefix.join("lib/node_modules").join(r.agent.package())
        || r.command != prefix.join("bin").join(r.agent.key())
        || !["succeeded", "installing", "needs_recovery"].contains(&r.state.as_str())
    {
        return Err(err("INSTALL_ID"));
    }
    for p in [prefix.join("bin"), r.package.parent().unwrap().into()] {
        directory(&p)?;
    }
    if fs::symlink_metadata(&r.command).is_ok() {
        let target = fs::read_link(&r.command).map_err(|_| err("INSTALL_CHANGED"))?;
        if !target.starts_with(&r.package) {
            return Err(err("INSTALL_CHANGED"));
        }
    }
    if r.package.exists() {
        snapshot::verify_tree(&r.package, &r.installed)?;
    }
    let detected = Installation {
        id: r.id.clone(),
        agent_id: r.agent,
        command_path: r.command.clone(),
        resolved_command: fs::canonicalize(&r.command).unwrap_or_else(|_| r.command.clone()),
        package_root: Some(r.package.clone()),
        prefix: Some(prefix.clone()),
        node_path: None,
        version: Some(r.version.clone()),
        architecture: String::new(),
        source: "user_npm".into(),
        supported: true,
        version_verified: true,
        reason: None,
    };
    npm::check_running(&detected)?;
    r.state = "needs_recovery".into();
    save(&work, &r)?;
    if fs::symlink_metadata(&r.command).is_ok() {
        fs::remove_file(&r.command).map_err(|_| err("INSTALL_REMOVE"))?;
    }
    if r.package.exists() {
        fs::rename(&r.package, work.join("removed-package")).map_err(|_| err("INSTALL_REMOVE"))?;
    }
    r.state = "restored".into();
    save(&work, &r)?;
    Ok(r)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub agent: AgentId,
    pub version: String,
    pub state: String,
    pub command: PathBuf,
    pub package: PathBuf,
    pub failure_code: Option<String>,
    pub created_at: u64,
}
impl From<Record> for Summary {
    fn from(r: Record) -> Self {
        Self {
            id: r.id,
            agent: r.agent,
            version: r.version,
            state: r.state,
            command: r.command,
            package: r.package,
            failure_code: r.failure_code,
            created_at: r.created_at,
        }
    }
}
