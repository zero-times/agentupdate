use crate::{agents::AgentId, error::AppError, platform::process};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    pub id: String,
    pub agent_id: AgentId,
    pub command_path: PathBuf,
    pub resolved_command: PathBuf,
    pub package_root: Option<PathBuf>,
    pub prefix: Option<PathBuf>,
    pub node_path: Option<PathBuf>,
    pub version: Option<String>,
    pub architecture: String,
    pub source: String,
    pub supported: bool,
    pub version_verified: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryReport {
    pub instances: Vec<Installation>,
    pub warnings: Vec<String>,
}

pub struct DiscoveryContext {
    pub home: PathBuf,
    pub directories: Vec<PathBuf>,
}

impl DiscoveryContext {
    pub fn current() -> Result<Self, AppError> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| AppError::new("HOME_UNAVAILABLE", "无法确定当前用户目录。"))?;
        #[cfg(feature = "desktop-validation")]
        if let Some(directory) = std::env::var_os("AGENTUPDATE_TEST_DISCOVERY_DIR") {
            return Ok(Self {
                home,
                directories: vec![PathBuf::from(directory)],
            });
        }
        let mut directories: Vec<_> = std::env::var_os("PATH")
            .map(|x| {
                std::env::split_paths(&x)
                    .filter(|p| p.is_absolute())
                    .collect()
            })
            .unwrap_or_default();
        directories.extend([
            home.join(".npm-global/bin"),
            home.join(".local/bin"),
            home.join(".volta/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ]);
        for (base, suffix) in [
            (home.join(".nvm/versions/node"), "bin"),
            (
                home.join(".local/share/fnm/node-versions"),
                "installation/bin",
            ),
            (home.join(".fnm/node-versions"), "installation/bin"),
        ] {
            if let Ok(entries) = fs::read_dir(base) {
                for entry in entries.flatten().take(64) {
                    directories.push(entry.path().join(suffix));
                }
            }
        }
        directories.truncate(256);
        Ok(Self { home, directories })
    }
}

pub fn discover(
    context: &DiscoveryContext,
    extra_paths: &[PathBuf],
) -> Result<DiscoveryReport, AppError> {
    if extra_paths.len() > 32
        || extra_paths
            .iter()
            .any(|p| !p.is_absolute() || p.as_os_str().len() > 4096)
    {
        return Err(AppError::new(
            "INVALID_DISCOVERY_PATH",
            "补充路径必须为绝对路径，最多 32 项。",
        ));
    }
    let mut candidates = Vec::new();
    for directory in context
        .directories
        .iter()
        .chain(extra_paths.iter().filter(|p| p.is_dir()))
    {
        for id in [
            AgentId::Codex,
            AgentId::Claude,
            AgentId::Deepseek,
            AgentId::Gemini,
        ] {
            candidates.push((id, directory.join(id.key())));
        }
    }
    for file in extra_paths.iter().filter(|p| !p.is_dir()) {
        for id in [
            AgentId::Codex,
            AgentId::Claude,
            AgentId::Deepseek,
            AgentId::Gemini,
        ] {
            if file.file_name().and_then(|x| x.to_str()) == Some(id.key()) {
                candidates.push((id, file.clone()));
            }
        }
    }
    let mut seen = HashSet::new();
    let mut instances = Vec::new();
    let mut warnings = Vec::new();
    // A scan must allow both supported tools after a cold x64 dependency starts.
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    for (agent, command) in candidates {
        if std::time::Instant::now() >= deadline {
            warnings.push("扫描达到时间上限，部分实例未读取；请补充具体命令路径后重试。".into());
            break;
        }
        let Ok(resolved) = fs::canonicalize(&command) else {
            continue;
        };
        if !resolved.is_file() || !seen.insert((agent, resolved.clone())) {
            continue;
        }
        match inspect(context, agent, &command, &resolved) {
            Ok(instance) => instances.push(instance),
            Err(_) => warnings.push(format!("{} 的一个安装实例无法读取，已跳过。", agent.key())),
        }
    }
    for extra in extra_paths {
        if !extra.exists() {
            warnings.push(format!("补充路径不存在：{}", extra.display()));
        }
    }
    Ok(DiscoveryReport {
        instances,
        warnings,
    })
}

fn inspect(
    context: &DiscoveryContext,
    agent: AgentId,
    command: &Path,
    resolved: &Path,
) -> Result<Installation, AppError> {
    let id = format!(
        "{}-{:x}",
        agent.key(),
        Sha256::digest(resolved.as_os_str().as_encoded_bytes())
    );
    let mut instance = Installation {
        id,
        agent_id: agent,
        command_path: command.to_path_buf(),
        resolved_command: resolved.to_path_buf(),
        package_root: None,
        prefix: None,
        node_path: None,
        version: None,
        architecture: "unknown".into(),
        source: "unknown".into(),
        supported: false,
        version_verified: false,
        reason: Some("无法确认官方 npm 安装范围。".into()),
    };
    let resolved_text = resolved.to_string_lossy();
    if resolved_text.contains("/Cellar/") || resolved_text.contains("/Caskroom/") {
        instance.source = "homebrew".into();
        instance.reason = Some("此 Homebrew 安装方式暂不支持升级。".into());
        populate_native_version(context, &mut instance);
        crate::updater::native::enable(context, &mut instance);
        return Ok(instance);
    }
    let mut package = None;
    for ancestor in resolved.ancestors().skip(1).take(8) {
        let file = ancestor.join("package.json");
        if let Ok(metadata) = fs::metadata(&file) {
            if metadata.len() > 128 * 1024 {
                continue;
            }
            if let Ok(value) = fs::read(&file).and_then(|b| {
                serde_json::from_slice::<serde_json::Value>(&b).map_err(std::io::Error::other)
            }) {
                if value.get("name").and_then(|x| x.as_str()) == Some(agent.package()) {
                    package = Some((ancestor.to_path_buf(), value));
                    break;
                }
            }
        }
    }
    let Some((root, manifest)) = package else {
        if resolved_text.contains("/.local/") || resolved_text.contains("/Applications/") {
            instance.source = "native".into();
            instance.reason = Some("此原生安装方式暂不支持升级。".into());
        }
        populate_native_version(context, &mut instance);
        crate::updater::native::enable(context, &mut instance);
        return Ok(instance);
    };
    let Some(prefix) = root
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
    else {
        return Ok(instance);
    };
    // Both supported packages are scoped; only standard global npm layout is accepted.
    if root
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|x| x.to_str())
        != agent.package().split('/').next()
        || prefix.join("lib/node_modules").join(agent.package()) != root
    {
        return Ok(instance);
    }
    let expected = manifest
        .get("bin")
        .and_then(|v| v.get(agent.key()))
        .and_then(|v| v.as_str())
        .or_else(|| manifest.get("bin").and_then(|v| v.as_str()));
    if expected
        .and_then(|p| fs::canonicalize(root.join(p)).ok())
        .as_deref()
        != Some(resolved)
    {
        instance.reason = Some("启动路径与包声明不一致。".into());
        return Ok(instance);
    }
    let declared = manifest
        .get("version")
        .and_then(|v| v.as_str())
        .and_then(|v| semver::Version::parse(v).ok())
        .map(|v| v.to_string());
    instance.version = declared;
    instance.package_root = Some(root.clone());
    instance.prefix = Some(prefix.to_path_buf());
    let writable = user_owned_writable(prefix) && user_owned_writable(&root);
    let protected = ["/usr", "/System", "/Library", "/opt/homebrew"]
        .iter()
        .any(|p| prefix.starts_with(p));
    let owned_brew_npm =
        writable && [Path::new("/opt/homebrew"), Path::new("/usr/local")].contains(&prefix);
    if !writable || (protected && !owned_brew_npm) {
        instance.source = "system_npm".into();
        instance.reason = Some("系统级或非当前用户所有的 npm 安装仅展示，不开放修改。".into());
    } else {
        instance.source = "user_npm".into();
    }
    let node = std::iter::once(prefix.join("bin/node"))
        .chain(context.directories.iter().map(|p| p.join("node")))
        .find(|p| p.is_file());
    let Some(node) = node.and_then(|p| fs::canonicalize(p).ok()) else {
        instance.reason = Some("找不到此安装可用的 Node.js。".into());
        return Ok(instance);
    };
    instance.node_path = Some(node.clone());
    match probe(context, &node, resolved) {
        Ok((version, architecture)) if instance.version.as_deref() == Some(&version) => {
            instance.version = Some(version);
            instance.architecture = architecture;
            instance.version_verified = true;
            instance.supported = writable && (!protected || owned_brew_npm);
            if instance.supported {
                instance.reason = None;
            }
        }
        Ok(_) => instance.reason = Some("实际命令版本与 npm 包声明不一致。".into()),
        Err(error) => instance.reason = Some(error.message),
    }
    Ok(instance)
}

fn user_owned_writable(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path)
        .map(|m| m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o200 != 0)
        .unwrap_or(false)
}

pub fn probe(
    context: &DiscoveryContext,
    node: &Path,
    launcher: &Path,
) -> Result<(String, String), AppError> {
    verify(context, node, launcher, false)
}

pub fn verify(
    context: &DiscoveryContext,
    node: &Path,
    launcher: &Path,
    health: bool,
) -> Result<(String, String), AppError> {
    verify_executable(context, Some(node), launcher, health)
}

fn populate_native_version(context: &DiscoveryContext, instance: &mut Installation) {
    match verify_executable(context, None, &instance.resolved_command, false) {
        Ok((version, architecture)) => {
            instance.version = Some(version);
            instance.architecture = architecture;
            instance.version_verified = true;
        }
        Err(error) => {
            instance.reason = Some(format!(
                "{} 版本读取失败：{}",
                instance.reason.as_deref().unwrap_or("此来源仅展示。"),
                error.message
            ));
        }
    }
}

pub fn verify_native(
    context: &DiscoveryContext,
    launcher: &Path,
    health: bool,
) -> Result<(String, String), AppError> {
    verify_executable(context, None, launcher, health)
}

fn verify_executable(
    context: &DiscoveryContext,
    node: Option<&Path>,
    launcher: &Path,
    health: bool,
) -> Result<(String, String), AppError> {
    use std::os::unix::fs::PermissionsExt;
    let directory =
        std::env::temp_dir().join(format!("agentupdate-probe-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory)
        .map_err(|_| AppError::new("PROBE_DIRECTORY", "无法建立隔离检查目录。"))?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .map_err(|_| AppError::new("PROBE_DIRECTORY", "无法限制隔离目录权限。"))?;
    let outcome = (|| {
        let quote = |p: &Path| serde_json::to_string(&p.to_string_lossy()).unwrap();
        let mut profile = format!("(version 1)(allow default)(deny network*)(deny file-write*)(allow file-write* (subpath {}))", quote(&directory));
        for path in [
            ".codex",
            ".gemini",
            ".claude",
            ".claude.json",
            ".dsh",
            ".ssh",
            ".npmrc",
            "Library/Keychains",
        ] {
            profile.push_str(&format!(
                "(deny file-read* (subpath {}))",
                quote(&context.home.join(path))
            ));
        }
        let env = vec![
            ("HOME".into(), directory.to_string_lossy().into_owned()),
            ("DSH_HOME".into(), directory.to_string_lossy().into_owned()),
            (
                "GEMINI_CLI_HOME".into(),
                directory.to_string_lossy().into_owned(),
            ),
            (
                "PATH".into(),
                format!(
                    "{}:/usr/bin:/bin:/usr/sbin:/sbin",
                    node.and_then(Path::parent)
                        .unwrap_or(Path::new("/usr/bin"))
                        .display()
                ),
            ),
            ("TMPDIR".into(), directory.to_string_lossy().into_owned()),
            ("CI".into(), "1".into()),
            ("NO_COLOR".into(), "1".into()),
        ];
        let execute = |exe: &Path, arguments: &[&str], timeout: Duration| {
            let mut args = vec![
                "-p".into(),
                profile.clone(),
                exe.to_string_lossy().into_owned(),
            ];
            args.extend(arguments.iter().map(|s| s.to_string()));
            process::run(
                Path::new("/usr/bin/sandbox-exec"),
                &args,
                &directory,
                &env,
                timeout,
                64 * 1024,
            )
        };
        let arch = if let Some(node) = node {
            execute(node, &["-p", "process.arch"], Duration::from_secs(12))?
        } else {
            let slices = execute(
                Path::new("/usr/bin/lipo"),
                &[
                    "-archs",
                    launcher
                        .to_str()
                        .ok_or_else(|| AppError::new("AGENT_PATH", "路径格式无效。"))?,
                ],
                Duration::from_secs(5),
            )
            .unwrap_or_default();
            if slices
                .split_whitespace()
                .any(|a| a == std::env::consts::ARCH)
            {
                if std::env::consts::ARCH == "aarch64" {
                    "arm64".into()
                } else {
                    "x64".into()
                }
            } else if slices.contains("arm64") {
                "arm64".into()
            } else if slices.contains("x86_64") {
                "x64".into()
            } else {
                "unknown".into()
            }
        };
        let arch = arch.trim();
        if node.is_some() && !["arm64", "x64"].contains(&arch) {
            return Err(AppError::new("UNSUPPORTED_ARCH", "Node.js 架构不受支持。"));
        }
        if let Some(node) = node {
            let node_version = execute(node, &["--version"], Duration::from_secs(12))?;
            let node_version = semver::Version::parse(node_version.trim().trim_start_matches('v'))
                .map_err(|_| AppError::new("NODE_VERSION", "无法确认 Node.js 版本。"))?;
            if node_version < semver::Version::new(22, 12, 0) {
                return Err(AppError::new(
                    "NODE_VERSION",
                    "管理 npm 安装需要 Node.js 22.12 或更高版本。",
                ));
            }
        }
        // A fresh x64 native dependency can require Rosetta translation before it starts.
        // Keep this bounded while allowing the observed ~31s cold start to finish.
        let launcher_timeout = Duration::from_secs(if arch == "x64" { 60 } else { 12 });
        let raw = execute(launcher, &["--version"], launcher_timeout)?;
        let version = raw
            .split_whitespace()
            .find_map(|s| semver::Version::parse(s).ok())
            .ok_or_else(|| AppError::new("AGENT_VERSION", "无法确认工具实际版本。"))?;
        if health {
            let help = execute(launcher, &["--help"], launcher_timeout)?;
            if help.trim().is_empty() {
                return Err(AppError::new("HEALTH_FAILED", "工具帮助启动检查失败。"));
            }
        }
        Ok((version.to_string(), arch.to_string()))
    })();
    let _ = fs::remove_dir_all(&directory);
    outcome
}
