use crate::{
    agents::{
        channels,
        discovery::{self, DiscoveryContext, Installation},
        versions::{AvailableVersion, VersionCatalog},
        AgentId,
    },
    backup::snapshot,
    error::AppError,
    platform::{files, process},
    storage::model::UpdateTransaction,
};
use base64::Engine;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

pub fn handles(i: &Installation) -> bool {
    (i.agent_id == AgentId::Claude && ["homebrew", "user_native"].contains(&i.source.as_str()))
        || (i.agent_id == AgentId::Codex && i.source == "homebrew")
}
fn owned(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| {
        m.is_dir()
            && !m.file_type().is_symlink()
            && m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o200 != 0
    })
}
pub fn enable(ctx: &DiscoveryContext, i: &mut Installation) {
    if ![AgentId::Claude, AgentId::Codex].contains(&i.agent_id)
        || !i.version_verified
        || !["arm64", "x64"].contains(&i.architecture.as_str())
    {
        return;
    }
    let resolved = &i.resolved_command;
    let layout = if i.source == "homebrew" {
        resolved
            .ancestors()
            .find(|p| {
                p.file_name().is_some_and(|n| n == cask_token(i.agent_id))
                    && p.parent()
                        .is_some_and(|p| p.file_name().is_some_and(|n| n == "Caskroom"))
            })
            .and_then(|root| {
                Some((
                    root.to_path_buf(),
                    root.parent()?.parent()?.to_path_buf(),
                    "homebrew",
                ))
            })
    } else if i.agent_id == AgentId::Claude {
        let prefix = fs::canonicalize(&ctx.home)
            .unwrap_or_else(|_| ctx.home.clone())
            .join(".local");
        let root = prefix.join("share/claude/versions");
        (resolved.parent() == Some(root.as_path())
            && resolved.file_name().and_then(|n| n.to_str()) == i.version.as_deref())
        .then_some((root, prefix, "user_native"))
    } else {
        None
    };
    if let Some((root, prefix, source)) = layout {
        if owned(&root)
            && owned(&prefix)
            && fs::canonicalize(i.command_path.parent().unwrap()).ok() == Some(prefix.join("bin"))
            && (source != "homebrew"
                || ([Path::new("/opt/homebrew"), Path::new("/usr/local")]
                    .contains(&prefix.as_path())
                    && prefix.join("bin/brew").is_file()))
            && fs::read_link(&i.command_path).is_ok()
        {
            i.package_root = Some(root);
            i.prefix = Some(prefix);
            i.source = source.into();
            i.supported = true;
            i.reason = None;
        }
    }
}
pub fn context(i: &Installation) -> Result<super::npm::Context, AppError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| AppError::new("HOME_UNAVAILABLE", "无法确认用户目录。"))?;
    Ok(super::npm::Context {
        home,
        npm: i
            .prefix
            .as_ref()
            .ok_or_else(|| AppError::new("NATIVE_PATH", "原生安装目录缺失。"))?
            .join("bin/brew"),
        cache: None,
        offline: false,
    })
}
pub fn validate_paths(i: &Installation) -> Result<(), AppError> {
    let prefix = i
        .prefix
        .as_ref()
        .ok_or_else(|| AppError::new("NATIVE_PATH", "安装目录缺失。"))?;
    let root = i
        .package_root
        .as_ref()
        .ok_or_else(|| AppError::new("NATIVE_PATH", "安装材料目录缺失。"))?;
    if i.source == "homebrew"
        && ![Path::new("/opt/homebrew"), Path::new("/usr/local")].contains(&prefix.as_path())
    {
        return Err(AppError::new(
            "NATIVE_PATH",
            "此 Homebrew 目录未纳入可信更新范围。",
        ));
    }
    let expected = if i.source == "homebrew" {
        prefix.join("Caskroom").join(cask_token(i.agent_id))
    } else {
        prefix.join("share/claude/versions")
    };
    if !handles(i)
        || *root != expected
        || i.command_path.file_name().and_then(|n| n.to_str()) != Some(i.agent_id.key())
        || fs::canonicalize(i.command_path.parent().unwrap()).ok() != Some(prefix.join("bin"))
        || !i.resolved_command.starts_with(root)
    {
        return Err(AppError::new(
            "NATIVE_PATH",
            "原生恢复范围与已识别安装不一致。",
        ));
    }
    for p in [
        prefix.as_path(),
        root.parent().unwrap(),
        prefix.join("bin").as_path(),
    ] {
        if !owned(p) || fs::canonicalize(p).ok().as_deref() != Some(p) {
            return Err(AppError::new("NATIVE_PATH", "安装目录权限或链接已变化。"));
        }
    }
    if fs::symlink_metadata(root).is_ok_and(|m| !m.is_dir() || m.file_type().is_symlink())
        || fs::symlink_metadata(&i.command_path).is_ok_and(|m| !m.file_type().is_symlink())
    {
        return Err(AppError::new(
            "NATIVE_PATH",
            "原生目录或启动链接的类型已变化。",
        ));
    }
    Ok(())
}
pub fn catalog(i: &Installation) -> Result<VersionCatalog, AppError> {
    if i.source == "homebrew" {
        return if i.agent_id == AgentId::Codex {
            codex_cask(&channels::cask("codex")?, &i.architecture)
        } else {
            from_cask(&channels::cask("claude-code")?, &i.architecture)
        };
    }
    let raw = channels::claude_document("claude-code-releases/latest")?;
    let version = std::str::from_utf8(&raw)
        .ok()
        .map(str::trim)
        .and_then(|v| semver::Version::parse(v).ok())
        .filter(|v| v.pre.is_empty())
        .ok_or_else(|| AppError::new("NATIVE_CHANNEL", "Claude 最新版本响应无效。"))?
        .to_string();
    let bytes =
        channels::claude_document(&format!("claude-code-releases/{version}/manifest.json"))?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::new("NATIVE_CHANNEL", "Claude 制品清单无效。"))?;
    from_manifest(&manifest, &version, &i.architecture)
}
pub fn from_manifest(
    manifest: &serde_json::Value,
    version: &str,
    arch: &str,
) -> Result<VersionCatalog, AppError> {
    let error = || AppError::new("NATIVE_CHANNEL", "Claude 制品清单版本、架构或校验无效。");
    let parsed = semver::Version::parse(version).map_err(|_| error())?;
    if !parsed.pre.is_empty() || manifest["version"].as_str() != Some(version) {
        return Err(error());
    }
    let platform = match arch {
        "arm64" => "darwin-arm64",
        "x64" => "darwin-x64",
        _ => return Err(error()),
    };
    let hex = manifest["platforms"][platform]["checksum"]
        .as_str()
        .ok_or_else(error)?;
    if manifest["platforms"][platform]["binary"] != "claude"
        || hex.len() != 64
        || !hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(error());
    }
    let bytes: Vec<u8> = (0..64)
        .step_by(2)
        .map(|n| u8::from_str_radix(&hex[n..n + 2], 16).unwrap())
        .collect();
    Ok(VersionCatalog {
        agent_id: AgentId::Claude,
        latest_stable: version.into(),
        versions: vec![AvailableVersion {
            version: version.into(),
            prerelease: false,
            tarball: format!(
                "https://downloads.claude.ai/claude-code-releases/{version}/{platform}/claude"
            ),
            integrity: format!(
                "sha256-{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
        }],
    })
}
pub fn from_cask(value: &serde_json::Value, arch: &str) -> Result<VersionCatalog, AppError> {
    let error = || {
        AppError::new(
            "NATIVE_CHANNEL",
            "Claude 官方原生制品的版本、架构或校验无法确认。",
        )
    };
    if value["token"].as_str() != Some("claude-code") || value["disabled"] == true {
        return Err(error());
    }
    let version = value["version"].as_str().ok_or_else(error)?;
    let parsed = semver::Version::parse(version).map_err(|_| error())?;
    if !parsed.pre.is_empty() {
        return Err(error());
    }
    let platform = match arch {
        "arm64" => "darwin-arm64",
        "x64" => "darwin-x64",
        _ => return Err(error()),
    };
    let expected =
        format!("https://downloads.claude.ai/claude-code-releases/{version}/{platform}/claude");
    let metadata = std::iter::once(value)
        .chain(
            value["variations"]
                .as_object()
                .into_iter()
                .flat_map(|v| v.values()),
        )
        .find(|v| v["url"].as_str() == Some(&expected))
        .ok_or_else(error)?;
    let hex = metadata["sha256"].as_str().ok_or_else(error)?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error());
    }
    let bytes: Vec<u8> = (0..64)
        .step_by(2)
        .map(|n| u8::from_str_radix(&hex[n..n + 2], 16).unwrap())
        .collect();
    // This adapter only permits the simple binary cask, without lifecycle scripts/installers.
    let artifacts = value["artifacts"].as_array().ok_or_else(error)?;
    if artifacts
        .iter()
        .filter(|a| {
            a["binary"]
                .as_array()
                .is_some_and(|v| v.len() == 1 && v[0] == "claude")
        })
        .count()
        != 1
        || artifacts.iter().any(|a| {
            a.as_object().is_none_or(|o| {
                if o.contains_key("binary") {
                    o.keys()
                        .any(|key| !["binary", "target"].contains(&key.as_str()))
                        || a["binary"] != serde_json::json!(["claude"])
                        || o.get("target").is_some_and(|target| {
                            target.as_str() != Some("$HOMEBREW_PREFIX/bin/claude")
                        })
                } else {
                    o.len() != 1 || !o.contains_key("zap")
                }
            })
        })
    {
        return Err(error());
    }
    Ok(VersionCatalog {
        agent_id: AgentId::Claude,
        latest_stable: version.into(),
        versions: vec![AvailableVersion {
            version: version.into(),
            prerelease: false,
            tarball: expected,
            integrity: format!(
                "sha256-{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
        }],
    })
}
pub fn validate_source(i: &Installation, target: &AvailableVersion) -> Result<(), AppError> {
    if i.agent_id == AgentId::Codex {
        let arch = if i.architecture == "arm64" {
            "aarch64"
        } else {
            "x86_64"
        };
        if handles(i) && semver::Version::parse(&target.version).is_ok_and(|v|v.pre.is_empty()) && target.integrity.starts_with("sha256-") && target.tarball == format!("https://github.com/openai/codex/releases/download/rust-v{}/codex-package-{arch}-apple-darwin.tar.gz",target.version) { return Ok(()); }
        return Err(AppError::new(
            "ARTIFACT_SOURCE",
            "Codex 官方原生制品来源无效。",
        ));
    }
    let version = semver::Version::parse(&target.version)
        .map_err(|_| AppError::new("NATIVE_TARGET", "原生目标版本无效。"))?;
    let platform = match i.architecture.as_str() {
        "arm64" => "darwin-arm64",
        "x64" => "darwin-x64",
        _ => "invalid",
    };
    if !handles(i)
        || !version.pre.is_empty()
        || !target.integrity.starts_with("sha256-")
        || target.tarball
            != format!(
                "https://downloads.claude.ai/claude-code-releases/{version}/{platform}/claude"
            )
    {
        return Err(AppError::new(
            "ARTIFACT_SOURCE",
            "原生制品不是已认可的官方来源。",
        ));
    }
    Ok(())
}
pub fn preflight(i: &Installation, ctx: &super::npm::Context) -> Result<(), AppError> {
    validate_paths(i)?;
    super::npm::check_running(i)?;
    let report = discovery::discover(
        &DiscoveryContext {
            home: ctx.home.clone(),
            directories: vec![i.prefix.as_ref().unwrap().join("bin")],
        },
        &[],
    )?;
    if !report.instances.iter().any(|n| {
        n.id == i.id
            && n.source == i.source
            && n.supported
            && n.version == i.version
            && n.architecture == i.architecture
    }) {
        return Err(AppError::new(
            "INSTANCE_CHANGED",
            "工具版本、来源或权限已变化，请重新扫描。",
        ));
    }
    snapshot::require_space(
        snapshot::available_space(i.prefix.as_ref().unwrap())?,
        512 * 1024 * 1024,
    )
}
pub fn stage(
    tx: &UpdateTransaction,
    ctx: &super::npm::Context,
    work: &Path,
) -> Result<Installation, AppError> {
    files::private_directory(work)?;
    let binary = work.join("target.tgz"); // shared immutable download cache; contents are a native executable
    super::npm::download(&tx.installation, &tx.target, &binary)?;
    crate::progress::phase(crate::progress::Phase::Staging);
    let binary = if tx.installation.agent_id == AgentId::Codex {
        let extracted = work.join("codex-extracted");
        crate::artifacts::unpack(&binary, &extracted, false)?;
        extracted.join("bin/codex")
    } else {
        binary
    };
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700))
        .map_err(|_| AppError::new("NATIVE_STAGE", "无法设置隔离制品权限。"))?;
    let (version, arch) = discovery::verify_native(
        &DiscoveryContext {
            home: ctx.home.clone(),
            directories: vec![],
        },
        &binary,
        true,
    )?;
    if version != tx.target.version || arch != tx.installation.architecture {
        return Err(AppError::new(
            "NATIVE_STAGE",
            "隔离原生制品版本或架构不匹配。",
        ));
    }
    let mut staged = tx.installation.clone();
    staged.resolved_command = binary;
    staged.version = Some(version);
    Ok(staged)
}
pub fn install(
    tx: &UpdateTransaction,
    staged: &Installation,
    ctx: &super::npm::Context,
) -> Result<(), AppError> {
    let i = &tx.installation;
    validate_paths(i)?;
    if i.source == "homebrew" {
        let brew = i.prefix.as_ref().unwrap().join("bin/brew");
        if ctx.npm != brew {
            return Err(AppError::new("BREW_PATH", "Homebrew 路径与实例不匹配。"));
        }
        let mut env = vec![
            ("HOME".into(), ctx.home.display().to_string()),
            (
                "PATH".into(),
                format!(
                    "{}:/usr/bin:/bin:/usr/sbin:/sbin",
                    brew.parent().unwrap().display()
                ),
            ),
            ("HOMEBREW_NO_AUTO_UPDATE".into(), "1".into()),
            ("HOMEBREW_NO_INSTALL_CLEANUP".into(), "1".into()),
            ("HOMEBREW_NO_ANALYTICS".into(), "1".into()),
            ("NONINTERACTIVE".into(), "1".into()),
        ];
        if let Some(proxy) = crate::platform::network::proxy() {
            env.push(("HTTPS_PROXY".into(), proxy));
        }
        process::run(
            &brew,
            &[
                "upgrade".into(),
                "--cask".into(),
                cask_token(i.agent_id).into(),
            ],
            i.prefix.as_ref().unwrap(),
            &env,
            Duration::from_secs(600),
            2 * 1024 * 1024,
        )
        .map_err(|_| AppError::new("BREW_UPGRADE", "Homebrew 升级未确认成功；恢复材料已保留。"))?;
    } else {
        let dest = i.package_root.as_ref().unwrap().join(&tx.target.version);
        if dest.exists() {
            return Err(AppError::new(
                "NATIVE_CONFLICT",
                "目标原生版本文件已存在，请重新扫描或手动检查。",
            ));
        }
        let bytes = fs::read(&staged.resolved_command)
            .map_err(|_| AppError::new("NATIVE_STAGE", "隔离原生制品无法读取。"))?;
        super::npm::verify_integrity(&bytes, &tx.target.integrity)?;
        files::write_atomic(&dest, &bytes)?;
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))
            .map_err(|_| AppError::new("NATIVE_INSTALL", "无法设置新原生版本权限。"))?;
        super::npm::replace_link(&i.command_path, &dest)?;
    }
    Ok(())
}
pub fn verify(
    i: &Installation,
    executable: &Path,
    home: &Path,
    health: bool,
) -> Result<(String, String), AppError> {
    let ctx = DiscoveryContext {
        home: home.into(),
        directories: vec![],
    };
    if handles(i) {
        discovery::verify_native(&ctx, executable, health)
    } else {
        discovery::verify(
            &ctx,
            i.node_path
                .as_ref()
                .ok_or_else(|| AppError::new("NODE_UNAVAILABLE", "原 Node 不可用。"))?,
            executable,
            health,
        )
    }
}
pub fn verify_installed(
    tx: &UpdateTransaction,
    ctx: &super::npm::Context,
    work: &Path,
) -> Result<(), AppError> {
    let i = &tx.installation;
    let executable = fs::canonicalize(&i.command_path)
        .map_err(|_| AppError::new("VERIFY_PATH", "工具启动链接无法读取。"))?;
    if !executable.starts_with(i.package_root.as_ref().unwrap()) {
        return Err(AppError::new("VERIFY_PATH", "工具命令已离开受管范围。"));
    }
    let bytes =
        fs::read(&executable).map_err(|_| AppError::new("VERIFY_PATH", "原生安装无法读取。"))?;
    if i.agent_id == AgentId::Codex {
        let extracted = work.join("codex-extracted");
        let staged = extracted.join("bin/codex");
        if crate::artifacts::sha256(&staged)? != crate::artifacts::sha256(&executable)? {
            return Err(AppError::new(
                "ARTIFACT_INTEGRITY",
                "Homebrew 安装的 Codex 与已校验制品不一致。",
            ));
        }
        let expected = snapshot::scan(&extracted, None)?;
        let installed = snapshot::scan(executable.parent().unwrap().parent().unwrap(), None)?;
        if expected
            .entries
            .iter()
            .filter(|e| e.kind == "file" || e.kind == "link")
            .any(|e| !installed.entries.contains(e))
        {
            return Err(AppError::new(
                "ARTIFACT_INTEGRITY",
                "Homebrew 安装的 Codex 运行文件与已校验制品不一致。",
            ));
        }
    } else {
        super::npm::verify_integrity(&bytes, &tx.target.integrity)?;
    }
    let (version, arch) = verify(i, &executable, &ctx.home, true)?;
    if version != tx.target.version || arch != i.architecture {
        return Err(AppError::new(
            "VERIFY_VERSION",
            "工具实际版本或架构与目标不一致。",
        ));
    }
    Ok(())
}

fn cask_token(agent: AgentId) -> &'static str {
    if agent == AgentId::Codex {
        "codex"
    } else {
        "claude-code"
    }
}
pub fn codex_cask(value: &serde_json::Value, arch: &str) -> Result<VersionCatalog, AppError> {
    let fail = || {
        AppError::new(
            "NATIVE_CHANNEL",
            "Codex Homebrew 制品身份、架构或校验无效。",
        )
    };
    let version = value["version"].as_str().ok_or_else(fail)?;
    if value["token"] != "codex"
        || value["disabled"] == true
        || !semver::Version::parse(version).is_ok_and(|v| v.pre.is_empty())
    {
        return Err(fail());
    }
    let platform = match arch {
        "arm64" => "aarch64",
        "x64" => "x86_64",
        _ => return Err(fail()),
    };
    let url = format!("https://github.com/openai/codex/releases/download/rust-v{version}/codex-package-{platform}-apple-darwin.tar.gz");
    let metadata = std::iter::once(value)
        .chain(
            value["variations"]
                .as_object()
                .into_iter()
                .flat_map(|o| o.values()),
        )
        .find(|v| v["url"] == url)
        .ok_or_else(fail)?;
    let hex = metadata["sha256"].as_str().ok_or_else(fail)?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(fail());
    }
    let artifacts = value["artifacts"].as_array().ok_or_else(fail)?;
    if artifacts
        .iter()
        .filter(|a| {
            a["binary"] == serde_json::json!(["bin/codex"])
                && a["target"] == "$HOMEBREW_PREFIX/bin/codex"
        })
        .count()
        != 1
        || artifacts.iter().any(|a| {
            a.as_object().is_none_or(|o| {
                o.keys().any(|key| {
                    ![
                        "binary",
                        "target",
                        "generate_completions_from_executable",
                        "zap",
                    ]
                    .contains(&key.as_str())
                }) || o
                    .get("binary")
                    .is_some_and(|b| *b != serde_json::json!(["bin/codex"]))
                    || o.get("target")
                        .is_some_and(|t| t != "$HOMEBREW_PREFIX/bin/codex")
                    || o.get("generate_completions_from_executable")
                        .is_some_and(|c| {
                            c.as_array().is_none_or(|c| {
                                c.len() != 3 || c[0] != "bin/codex" || c[1] != "completion"
                            })
                        })
            })
        })
    {
        return Err(fail());
    }
    let bytes: Vec<_> = (0..64)
        .step_by(2)
        .map(|n| u8::from_str_radix(&hex[n..n + 2], 16).unwrap())
        .collect();
    Ok(VersionCatalog {
        agent_id: AgentId::Codex,
        latest_stable: version.into(),
        versions: vec![AvailableVersion {
            version: version.into(),
            prerelease: false,
            tarball: url,
            integrity: format!(
                "sha256-{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
        }],
    })
}
