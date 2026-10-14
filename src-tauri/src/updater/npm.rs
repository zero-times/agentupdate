use crate::{
    agents::{
        discovery::{self, DiscoveryContext, Installation},
        versions::AvailableVersion,
    },
    backup::snapshot,
    error::AppError,
    platform::{files, process},
    storage::model::UpdateTransaction,
};
use base64::Engine;
use sha2::{Digest, Sha512};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone)]
pub struct Context {
    pub home: PathBuf,
    pub npm: PathBuf,
    pub cache: Option<PathBuf>,
    pub offline: bool,
}
impl Context {
    pub fn current(installation: &Installation) -> Result<Self, AppError> {
        if super::native::handles(installation) {
            return super::native::context(installation);
        }
        let node = installation
            .node_path
            .as_ref()
            .ok_or_else(|| err("NODE_UNAVAILABLE", "没有可用 Node.js。"))?;
        let npm = fs::canonicalize(node.parent().unwrap().join("npm")).map_err(|_| {
            err(
                "NPM_UNAVAILABLE",
                "找不到此 Node.js 配套的 npm，请安装匹配的 npm。",
            )
        })?;
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| err("HOME_UNAVAILABLE", "无法确认用户目录。"))?;
        let context = Self {
            home,
            npm,
            cache: None,
            offline: false,
        };
        #[cfg(feature = "desktop-validation")]
        let context = Self {
            cache: std::env::var_os("AGENTUPDATE_NPM_CACHE").map(PathBuf::from),
            offline: std::env::var_os("AGENTUPDATE_TEST_OFFLINE").is_some(),
            ..context
        };
        Ok(context)
    }
}
fn err(code: &'static str, message: &str) -> AppError {
    AppError::new(code, message)
}
pub fn process_uses_instance(command: &str, instance: &Installation) -> bool {
    let command = command.trim();
    let matches_command = |path: &Path| {
        let path = path.to_string_lossy();
        command
            .strip_prefix(path.as_ref())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
    };
    if matches_command(&instance.command_path)
        || matches_command(&instance.resolved_command)
        || instance
            .package_root
            .as_ref()
            .is_some_and(|p| command.starts_with(&format!("{}/", p.display())))
    {
        return true;
    }
    if let Some(node) = &instance.node_path {
        let node = node.to_string_lossy();
        if let Some(rest) = command
            .strip_prefix(node.as_ref())
            .filter(|s| s.starts_with(char::is_whitespace))
        {
            return [&instance.command_path, &instance.resolved_command]
                .iter()
                .any(|p| {
                    let text = p.to_string_lossy();
                    rest.match_indices(text.as_ref()).any(|(offset, _)| {
                        let before = &rest[..offset];
                        let after = &rest[offset + text.len()..];
                        (before.is_empty() || before.ends_with(char::is_whitespace))
                            && (after.is_empty() || after.starts_with(char::is_whitespace))
                    })
                });
        }
    }
    let first = command.split_whitespace().next().unwrap_or("");
    let name = Path::new(first)
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or("");
    if first == instance.agent_id.key()
        || first == instance.command_path.to_string_lossy()
        || first == instance.resolved_command.to_string_lossy()
        || instance
            .package_root
            .as_ref()
            .is_some_and(|p| Path::new(first).starts_with(p))
    {
        return true;
    }
    if name == "node" {
        let rest = command.strip_prefix(first).unwrap_or("").trim_start();
        return [
            Some(&instance.command_path),
            Some(&instance.resolved_command),
        ]
        .into_iter()
        .flatten()
        .any(|p| rest.starts_with(&p.to_string_lossy().to_string()));
    }
    false
}
pub fn check_running(instance: &Installation) -> Result<(), AppError> {
    let output = process::run(
        Path::new("/bin/ps"),
        &["-axo".into(), "command=".into()],
        Path::new("/"),
        &[],
        Duration::from_secs(5),
        2 * 1024 * 1024,
    )?;
    if output
        .lines()
        .any(|line| process_uses_instance(line, instance))
    {
        return Err(err(
            "AGENT_RUNNING",
            "工具正在运行，请先退出相关工具，再重试。",
        ));
    }
    Ok(())
}
pub fn preflight(instance: &Installation, context: &Context) -> Result<(), AppError> {
    if !instance.supported || instance.source != "user_npm" {
        return Err(err("UNSUPPORTED_INSTALLATION", "该安装来源不能安全更新。"));
    }
    check_running(instance)?;
    let report = discovery::discover(
        &DiscoveryContext {
            home: context.home.clone(),
            directories: vec![
                instance
                    .prefix
                    .as_ref()
                    .ok_or_else(|| err("INSTANCE_UNAVAILABLE", "安装目录无效。"))?
                    .join("bin"),
                instance
                    .node_path
                    .as_ref()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .into(),
            ],
        },
        &[],
    )?;
    let current = report
        .instances
        .iter()
        .find(|i| i.id == instance.id)
        .ok_or_else(|| err("INSTANCE_CHANGED", "实例命令路径已变化，请重新扫描。"))?;
    if !current.supported
        || current.version != instance.version
        || current.node_path != instance.node_path
        || current.architecture != instance.architecture
    {
        return Err(err(
            "INSTANCE_CHANGED",
            "实例版本、架构或权限已变化，请重新扫描。",
        ));
    }
    let root = instance.package_root.as_ref().unwrap();
    let prefix = instance.prefix.as_ref().unwrap();
    if fs::canonicalize(root).ok().as_ref() != Some(root)
        || fs::canonicalize(prefix).ok().as_ref() != Some(prefix)
    {
        return Err(err("INSTANCE_CHANGED", "安装路径存在链接或已变化。"));
    }
    snapshot::require_space(snapshot::available_space(prefix)?, 512 * 1024 * 1024)?;
    Ok(())
}
pub fn verify_integrity(bytes: &[u8], integrity: &str) -> Result<(), AppError> {
    if let Some(encoded) = integrity.strip_prefix("sha256-") {
        let expected = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| err("ARTIFACT_INTEGRITY", "目标制品校验标识无效。"))?;
        if expected.len() != 32 || sha2::Sha256::digest(bytes).as_slice() != expected {
            return Err(err("ARTIFACT_INTEGRITY", "目标制品完整性校验失败。"));
        }
        return Ok(());
    }
    let expected = integrity
        .strip_prefix("sha512-")
        .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
        .ok_or_else(|| err("ARTIFACT_INTEGRITY", "目标制品校验标识无效。"))?;
    if expected.len() != 64 || Sha512::digest(bytes).as_slice() != expected {
        return Err(err("ARTIFACT_INTEGRITY", "目标制品完整性校验失败。"));
    }
    Ok(())
}
pub fn validate_source(instance: &Installation, target: &AvailableVersion) -> Result<(), AppError> {
    if super::native::handles(instance) {
        return super::native::validate_source(instance, target);
    }
    let version = semver::Version::parse(&target.version)
        .map_err(|_| err("INVALID_VERSION", "目标版本无效。"))?;
    let filename = instance.agent_id.package().split('/').next_back().unwrap();
    if target.tarball
        != format!(
            "https://registry.npmjs.org/{}/-/{}-{}.tgz",
            instance.agent_id.package(),
            filename,
            version
        )
    {
        return Err(err("ARTIFACT_SOURCE", "目标制品不是已认可的官方来源。"));
    }
    Ok(())
}
pub fn download(
    instance: &Installation,
    target: &AvailableVersion,
    path: &Path,
) -> Result<(), AppError> {
    validate_source(instance, target)?;
    crate::progress::phase(crate::progress::Phase::Downloading);
    if path.exists() {
        files::regular_private_file(path)?;
        let file = fs::File::open(path).map_err(|_| err("ARTIFACT_READ", "缓存制品无法读取。"))?;
        let mut bytes = Vec::new();
        file.take(256 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| err("ARTIFACT_READ", "缓存制品无法读取。"))?;
        if bytes.len() > 256 * 1024 * 1024 {
            return Err(err("ARTIFACT_SIZE", "缓存制品超出允许范围。"));
        }
        return verify_integrity(&bytes, &target.integrity);
    }
    if instance.agent_id == crate::agents::AgentId::Codex && super::native::handles(instance) {
        let digest = target
            .integrity
            .strip_prefix("sha256-")
            .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
            .filter(|v| v.len() == 32)
            .ok_or_else(|| err("ARTIFACT_INTEGRITY", "校验值无效。"))?;
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        return crate::artifacts::download(&target.tarball, &hex, path);
    }
    let client =
        crate::platform::network::client(Duration::from_secs(15), Duration::from_secs(90))?;
    let mut last = err("DOWNLOAD_FAILED", "目标制品下载失败，原安装保留。");
    for _ in 0..2 {
        match client.get(&target.tarball).send() {
            Ok(response) if response.status().is_success() => {
                let mut bytes = Vec::new();
                if response
                    .content_length()
                    .is_some_and(|n| n > 256 * 1024 * 1024)
                {
                    return Err(err("ARTIFACT_SIZE", "目标制品超出允许范围。"));
                }
                let total = response.content_length();
                if crate::progress::copy_download(response, &mut bytes, 256 * 1024 * 1024, total)
                    .is_err()
                {
                    last = err("DOWNLOAD_INTERRUPTED", "目标制品下载中断，原安装保留。");
                    continue;
                }
                if bytes.len() > 256 * 1024 * 1024 {
                    return Err(err("ARTIFACT_SIZE", "目标制品超出允许范围。"));
                }
                verify_integrity(&bytes, &target.integrity)?;
                files::write_atomic(path, &bytes)?;
                return Ok(());
            }
            Ok(_) => return Err(err("DOWNLOAD_HTTP", "官方制品暂不可获取，原安装保留。")),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
    Err(last)
}
pub fn stage(
    transaction: &UpdateTransaction,
    context: &Context,
    work: &Path,
) -> Result<Installation, AppError> {
    let instance = &transaction.installation;
    files::private_directory(work)?;
    let tarball = work.join("target.tgz");
    download(instance, &transaction.target, &tarball)?;
    crate::progress::phase(crate::progress::Phase::Staging);
    let prefix = work.join("prefix");
    files::private_directory(&prefix)?;
    let empty = work.join("empty.user.npmrc");
    files::write_atomic(&empty, b"")?;
    let global_config = work.join("empty.global.npmrc");
    files::write_atomic(&global_config, b"")?;
    let temporary_home = work.join("home");
    files::private_directory(&temporary_home)?;
    let node = instance.node_path.as_ref().unwrap();
    let cache = context
        .cache
        .clone()
        .unwrap_or_else(|| work.join("npm-cache"));
    files::private_directory(&cache)?;
    let env: Vec<(String, String)> = vec![
        ("HOME".into(), temporary_home.display().to_string()),
        ("TMPDIR".into(), work.display().to_string()),
        (
            "PATH".into(),
            format!(
                "{}:/usr/bin:/bin:/usr/sbin:/sbin",
                node.parent().unwrap().display()
            ),
        ),
        ("CI".into(), "1".into()),
        ("NO_COLOR".into(), "1".into()),
    ];
    let mut env = env;
    for key in [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "https_proxy",
        "http_proxy",
        "NO_PROXY",
        "no_proxy",
    ] {
        if let Ok(value) = std::env::var(key) {
            env.push((key.into(), value));
        }
    }
    if !env
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("HTTPS_PROXY"))
    {
        if let Some(proxy) = crate::platform::network::proxy() {
            env.push(("HTTPS_PROXY".into(), proxy));
        }
    }
    let mut profile = format!(
        "(version 1)(allow default)(deny file-write*)(allow file-write* (subpath {}))",
        serde_json::to_string(&work.to_string_lossy()).unwrap()
    );
    if cache != work.join("npm-cache") {
        profile.push_str(&format!(
            "(allow file-write* (subpath {}))",
            serde_json::to_string(&cache.to_string_lossy()).unwrap()
        ));
    }
    for secret in [
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
            serde_json::to_string(&context.home.join(secret).to_string_lossy()).unwrap()
        ));
    }
    let mut args = vec![
        "-p".into(),
        profile,
        node.display().to_string(),
        context.npm.display().to_string(),
        "install".into(),
        "--global".into(),
        "--prefix".into(),
        prefix.display().to_string(),
        "--ignore-scripts".into(),
        "--no-audit".into(),
        "--no-fund".into(),
        "--registry=https://registry.npmjs.org".into(),
        "--userconfig".into(),
        empty.display().to_string(),
        "--globalconfig".into(),
        global_config.display().to_string(),
        "--cache".into(),
        cache.display().to_string(),
        "--fetch-retries=1".into(),
        "--fetch-timeout=30000".into(),
        tarball.display().to_string(),
    ];
    if context.offline {
        args.push("--offline".into());
    }
    process::run(
        Path::new("/usr/bin/sandbox-exec"),
        &args,
        work,
        &env,
        Duration::from_secs(180),
        1024 * 1024,
    )
    .map_err(|_| {
        err(
            "STAGED_INSTALL_FAILED",
            "新版本隔离安装失败；原安装尚未修改。",
        )
    })?;
    prepare_claude_npm_binary(instance, &transaction.target, &prefix, &context.home)?;
    std::os::unix::fs::symlink(node, prefix.join("bin/node"))
        .map_err(|_| err("STAGE_SETUP", "无法关联已验证的 Node.js。"))?;
    let report = discovery::discover(
        &DiscoveryContext {
            home: context.home.clone(),
            directories: vec![prefix.join("bin")],
        },
        &[],
    )?;
    let staged = report
        .instances
        .into_iter()
        .find(|i| {
            i.agent_id == instance.agent_id
                && i.supported
                && i.version.as_deref() == Some(&transaction.target.version)
        })
        .ok_or_else(|| {
            err(
                "STAGED_VERSION_FAILED",
                "新制品版本或依赖检查失败，原安装尚未修改。",
            )
        })?;
    discovery::verify(
        &DiscoveryContext {
            home: context.home.clone(),
            directories: vec![],
        },
        node,
        &staged.resolved_command,
        true,
    )?;
    snapshot::scan(staged.package_root.as_ref().unwrap(), None)?;
    Ok(staged)
}
pub fn swap_package(
    transaction: &UpdateTransaction,
    staged: &Installation,
) -> Result<(), AppError> {
    let original = transaction.installation.package_root.as_ref().unwrap();
    let parent = original.parent().unwrap();
    let replacement = parent.join(format!(".agentupdate-new-{}", transaction.id));
    let displaced = parent.join(format!(".agentupdate-old-{}", transaction.id));
    if replacement.exists() || displaced.exists() {
        return Err(err(
            "SWAP_CONFLICT",
            "安装目录有未处理的更新材料，请先恢复。",
        ));
    }
    let snapshot = snapshot::scan(staged.package_root.as_ref().unwrap(), None)?;
    let archive = staged
        .prefix
        .as_ref()
        .unwrap()
        .parent()
        .unwrap()
        .join("publish-archive");
    snapshot::copy(staged.package_root.as_ref().unwrap(), &archive, &snapshot)?;
    snapshot::materialize(&archive, &snapshot, &replacement)?;
    files::sync_directory(parent)?;
    fs::rename(original, &displaced)
        .map_err(|_| err("INSTALL_SWAP", "无法保留原安装；需检查恢复。"))?;
    files::sync_directory(parent)?;
    #[cfg(feature = "desktop-validation")]
    if std::env::var("AGENTUPDATE_TEST_FAIL_SWAP_AGENT")
        .ok()
        .as_deref()
        == Some(transaction.installation.agent_id.key())
    {
        return Err(err(
            "INJECTED_INSTALL_FAILURE",
            "隔离测试：旧包保留后模拟安装失败。",
        ));
    }
    fs::rename(&replacement, original)
        .map_err(|_| err("INSTALL_SWAP", "新安装发布失败；需恢复原安装。"))?;
    files::sync_directory(parent)?;
    let relative = staged
        .resolved_command
        .strip_prefix(staged.package_root.as_ref().unwrap())
        .map_err(|_| err("LAUNCHER_PATH", "新启动路径不在受管包内。"))?;
    replace_link(
        &transaction.installation.command_path,
        &original.join(relative),
    )?;
    Ok(())
}
pub fn replace_link(destination: &Path, target: &Path) -> Result<(), AppError> {
    let temporary = destination
        .parent()
        .unwrap()
        .join(format!(".agentupdate-link-{}", uuid::Uuid::new_v4()));
    std::os::unix::fs::symlink(target, &temporary)
        .map_err(|_| err("LAUNCHER_WRITE", "无法建立启动链接。"))?;
    if let Err(_) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(temporary);
        return Err(err("LAUNCHER_WRITE", "无法发布启动链接。"));
    }
    files::sync_directory(destination.parent().unwrap())
}
pub fn verify_installed(
    transaction: &UpdateTransaction,
    context: &Context,
) -> Result<(), AppError> {
    #[cfg(feature = "desktop-validation")]
    if std::env::var("AGENTUPDATE_TEST_FAIL_VERIFY_AGENT")
        .ok()
        .as_deref()
        == Some(transaction.installation.agent_id.key())
    {
        return Err(err(
            "INJECTED_VERIFY_FAILURE",
            "隔离测试：模拟更新验证失败。",
        ));
    }
    let instance = &transaction.installation;
    let resolved = fs::canonicalize(&instance.command_path)
        .map_err(|_| err("VERIFY_PATH", "实际命令无法解析。"))?;
    if !resolved.starts_with(instance.package_root.as_ref().unwrap()) {
        return Err(err("VERIFY_PATH", "实际命令指向了其他安装。"));
    }
    let (version, arch) = discovery::verify(
        &DiscoveryContext {
            home: context.home.clone(),
            directories: vec![],
        },
        instance.node_path.as_ref().unwrap(),
        &resolved,
        true,
    )?;
    if version != transaction.target.version || arch != instance.architecture {
        return Err(err("VERIFY_VERSION", "实际版本或架构与目标不一致。"));
    }
    Ok(())
}

/// New Claude npm distributions carry an authenticated native optional dependency.
/// Reproduce the fixed binary placement without running package lifecycle scripts.
fn prepare_claude_npm_binary(
    instance: &Installation,
    target: &AvailableVersion,
    prefix: &Path,
    home: &Path,
) -> Result<(), AppError> {
    if instance.agent_id != crate::agents::AgentId::Claude {
        return Ok(());
    }
    let package = prefix.join("lib/node_modules/@anthropic-ai/claude-code");
    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(package.join("package.json"))
            .map_err(|_| err("CLAUDE_NPM_BINARY", "Claude npm 元数据无效。"))?,
    )
    .map_err(|_| err("CLAUDE_NPM_BINARY", "Claude npm 元数据无效。"))?;
    if metadata["bin"]["claude"] != "bin/claude.exe" {
        return Ok(());
    }
    let node = instance.node_path.as_ref().unwrap();
    let arch = process::run(
        node,
        &["-p".into(), "process.arch".into()],
        Path::new("/"),
        &[],
        Duration::from_secs(5),
        8192,
    )?;
    let arch = arch.trim();
    let platform = match arch {
        "arm64" => "darwin-arm64",
        "x64" => "darwin-x64",
        _ => return Err(err("CLAUDE_NPM_BINARY", "Claude npm 架构不支持。")),
    };
    let name = format!("@anthropic-ai/claude-code-{platform}");
    let dependency = package.join("node_modules").join(&name);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(dependency.join("package.json"))
            .map_err(|_| err("CLAUDE_NPM_BINARY", "Claude 官方平台制品未下载。"))?,
    )
    .map_err(|_| err("CLAUDE_NPM_BINARY", "Claude 平台元数据无效。"))?;
    if manifest["name"] != name || manifest["version"] != target.version {
        return Err(err("CLAUDE_NPM_BINARY", "Claude 平台包身份不一致。"));
    }
    let official = crate::agents::channels::claude_document(&format!(
        "claude-code-releases/{}/manifest.json",
        target.version
    ))?;
    let official: serde_json::Value = serde_json::from_slice(&official)
        .map_err(|_| err("CLAUDE_NPM_BINARY", "Claude 官方校验文档无效。"))?;
    let catalog = super::native::from_manifest(&official, &target.version, arch)?;
    let source = dependency.join("claude");
    let bytes =
        fs::read(&source).map_err(|_| err("CLAUDE_NPM_BINARY", "Claude 平台制品不可读。"))?;
    verify_integrity(&bytes, &catalog.versions[0].integrity)?;
    let (version, actual_arch) = discovery::verify_native(
        &DiscoveryContext {
            home: home.into(),
            directories: vec![],
        },
        &source,
        true,
    )?;
    if version != target.version || actual_arch != arch {
        return Err(err(
            "CLAUDE_NPM_BINARY",
            "Claude 平台制品实际版本或架构不一致。",
        ));
    }
    let binary = package.join("bin/claude.exe");
    files::write_atomic(&binary, &bytes)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))
        .map_err(|_| err("CLAUDE_NPM_BINARY", "Claude 平台制品无法设置执行权限。"))?;
    Ok(())
}
