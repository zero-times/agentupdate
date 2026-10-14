//! Headless bundle installer with durable snapshots and an out-of-process self updater.
use crate::{
    artifacts,
    backup::snapshot,
    desktop_apps::{self, Product},
    error::AppError,
    platform::{files, locks::OperationLock, process},
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BundleProduct {
    Agentupdate,
    Codex,
    Claude,
    Chatgpt,
}
impl BundleProduct {
    pub fn key(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Chatgpt => "chatgpt",
            Self::Agentupdate => "agentupdate",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Agentupdate => "AgentUpdate.app",
            Self::Codex => "Codex.app",
            Self::Claude => "Claude.app",
            Self::Chatgpt => "ChatGPT.app",
        }
    }
    pub fn bundle(self) -> &'static str {
        match self {
            Self::Agentupdate => "work.x5brain.agentupdate",
            Self::Codex => "com.openai.codex",
            Self::Claude => "com.anthropic.claudefordesktop",
            Self::Chatgpt => "com.openai.chat",
        }
    }
    pub fn desktop(self) -> Option<Product> {
        match self {
            Self::Agentupdate => None,
            Self::Codex => Some(Product::Codex),
            Self::Claude => Some(Product::Claude),
            Self::Chatgpt => Some(Product::Chatgpt),
        }
    }
    fn config(self) -> &'static [&'static str] {
        match self {
            Self::Codex => &[".codex/config.toml", ".codex/auth.json"],
            Self::Claude => &[
                "Library/Application Support/Claude/config.json",
                "Library/Preferences/com.anthropic.claudefordesktop.plist",
            ],
            Self::Chatgpt => &["Library/Preferences/com.openai.chat.plist"],
            Self::Agentupdate => &[],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub product: BundleProduct,
    pub version: String,
    pub destination: PathBuf,
    pub state: String,
    pub failure_code: Option<String>,
    pub created_at: u64,
    pub original: Option<snapshot::Snapshot>,
    pub staged: snapshot::Snapshot,
    pub config: snapshot::Snapshot,
    pub home: PathBuf,
}
#[derive(Clone, Debug)]
pub struct Package {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub zip: bool,
}
fn err(code: &'static str) -> AppError {
    AppError::new(
        code,
        "应用安装未确认完成，旧应用与恢复材料已保留。请查看安装记录。",
    )
}
fn directory(path: &Path) -> Result<(), AppError> {
    let m = fs::symlink_metadata(path).map_err(|_| err("APP_PATH"))?;
    if !m.is_dir()
        || m.file_type().is_symlink()
        || fs::canonicalize(path).ok().as_deref() != Some(path)
    {
        return Err(err("APP_PATH"));
    }
    Ok(())
}
fn writable(path: &Path) -> bool {
    use std::ffi::CString;
    CString::new(path.as_os_str().as_encoded_bytes())
        .is_ok_and(|p| unsafe { libc::access(p.as_ptr(), libc::W_OK) } == 0)
}
fn work(root: &Path, id: &str) -> Result<PathBuf, AppError> {
    uuid::Uuid::parse_str(id).map_err(|_| err("APP_INSTALL_ID"))?;
    Ok(root.join("app-installs").join(id))
}
fn save(work: &Path, r: &Record) -> Result<(), AppError> {
    files::write_atomic(
        &work.join("record.json"),
        &serde_json::to_vec(r).map_err(|_| err("APP_RECORD"))?,
    )
}
fn read(work: &Path) -> Result<Record, AppError> {
    let path = work.join("record.json");
    files::regular_private_file(&path)?;
    let bytes = fs::read(path).map_err(|_| err("APP_RECORD"))?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(err("APP_RECORD"));
    }
    serde_json::from_slice(&bytes).map_err(|_| err("APP_RECORD"))
}
pub fn history(root: &Path) -> Result<Vec<Record>, AppError> {
    let mut out = vec![];
    if let Ok(entries) = fs::read_dir(root.join("app-installs")) {
        for entry in entries.flatten().take(256) {
            if !entry.path().join("record.json").exists() {
                continue;
            }
            out.push(read(&entry.path())?);
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    Ok(out)
}
pub fn destination(home: &Path, product: BundleProduct) -> Result<PathBuf, AppError> {
    if product == BundleProduct::Agentupdate {
        let exe = std::env::current_exe().map_err(|_| err("APP_SELF_PATH"))?;
        let bundle = exe
            .ancestors()
            .find(|p| p.extension().is_some_and(|x| x == "app"))
            .ok_or_else(|| {
                AppError::new(
                    "APP_SELF_PATH",
                    "应用内升级需要安装到 /Applications 或用户 Applications 中。",
                )
            })?;
        validate_destination(home, product, bundle)?;
        return Ok(bundle.into());
    }
    let installed = desktop_apps::discover(home);
    let p = product.desktop().unwrap();
    let targets: Vec<_> = installed.iter().filter(|(n, _)| *n == p).collect();
    if targets.len() > 1 {
        return Err(AppError::new(
            "APP_MULTIPLE_INSTALLS",
            "发现多个应用实例，请保留一个标准 Applications 安装再升级。",
        ));
    }
    if let Some((_, app)) = targets.first() {
        validate_destination(home, product, &app.path)?;
        return Ok(app.path.clone());
    }
    let dir = if writable(Path::new("/Applications")) {
        PathBuf::from("/Applications")
    } else {
        let p = home.join("Applications");
        crate::cli_install::directory(&p)?;
        p
    };
    directory(&dir)?;
    Ok(dir.join(product.name()))
}
pub fn validate_destination(
    home: &Path,
    product: BundleProduct,
    destination: &Path,
) -> Result<(), AppError> {
    if destination.file_name() != Some(std::ffi::OsStr::new(product.name()))
        || ![PathBuf::from("/Applications"), home.join("Applications")]
            .iter()
            .any(|d| destination.parent() == Some(d.as_path()))
    {
        return Err(err("APP_DESTINATION"));
    }
    let parent = destination.parent().unwrap();
    directory(parent)?;
    if !writable(parent) {
        return Err(AppError::new(
            "APP_PERMISSION",
            "应用目录不可写，可将应用安装到用户 Applications 后重试。",
        ));
    }
    if let Ok(m) = fs::symlink_metadata(destination) {
        if !m.is_dir() || m.file_type().is_symlink() || m.uid() != unsafe { libc::geteuid() } {
            return Err(err("APP_DESTINATION"));
        }
        let v = plist(destination)?;
        if !identity(product, &v) {
            return Err(err("APP_IDENTITY"));
        }
    }
    Ok(())
}
pub fn plist(bundle: &Path) -> Result<serde_json::Value, AppError> {
    let path = bundle.join("Contents/Info.plist");
    if !fs::metadata(&path).is_ok_and(|m| m.len() < 512 * 1024) {
        return Err(err("APP_IDENTITY"));
    }
    let text = process::run(
        Path::new("/usr/bin/plutil"),
        &[
            "-convert".into(),
            "json".into(),
            "-o".into(),
            "-".into(),
            path.display().to_string(),
        ],
        Path::new("/"),
        &[],
        Duration::from_secs(5),
        512 * 1024,
    )?;
    serde_json::from_str(&text).map_err(|_| err("APP_IDENTITY"))
}
pub fn identity(product: BundleProduct, v: &serde_json::Value) -> bool {
    if v["CFBundleIdentifier"] != product.bundle()
        && !(product == BundleProduct::Chatgpt && v["CFBundleIdentifier"] == "com.openai.codex")
    {
        return false;
    }
    let chat = ["CFBundleName", "CFBundleDisplayName", "CFBundleExecutable"]
        .iter()
        .any(|key| v[key] == "ChatGPT");
    (product != BundleProduct::Codex || !chat) && (product != BundleProduct::Chatgpt || chat)
}
pub fn verify_bundle(
    bundle: &Path,
    product: BundleProduct,
    version: &str,
    arch: &str,
) -> Result<(), AppError> {
    let v = plist(bundle)?;
    if !identity(product, &v) || v["CFBundleShortVersionString"] != version {
        return Err(err("APP_IDENTITY"));
    }
    let executable = v["CFBundleExecutable"]
        .as_str()
        .filter(|s| !s.is_empty() && !s.contains('/') && !s.contains(".."))
        .ok_or_else(|| err("APP_EXECUTABLE"))?;
    let binary = bundle.join("Contents/MacOS").join(executable);
    let slices = process::run(
        Path::new("/usr/bin/lipo"),
        &["-archs".into(), binary.display().to_string()],
        Path::new("/"),
        &[],
        Duration::from_secs(5),
        8192,
    )?;
    let arch = if arch == "aarch64" { "arm64" } else { arch };
    if !slices.split_whitespace().any(|s| s == arch) {
        return Err(err("APP_ARCHITECTURE"));
    }
    process::run(
        Path::new("/usr/bin/codesign"),
        &[
            "--verify".into(),
            "--deep".into(),
            "--strict".into(),
            bundle.display().to_string(),
        ],
        Path::new("/"),
        &[],
        Duration::from_secs(90),
        16384,
    )
    .map_err(|_| err("APP_SIGNATURE"))?;
    if product != BundleProduct::Agentupdate {
        // codesign writes descriptive output to stderr, so capture through its -r test instead.
        let team = if product == BundleProduct::Claude {
            "Q6L2SF6YDW"
        } else {
            "2DC432GLL2"
        };
        process::run(
            Path::new("/usr/bin/codesign"),
            &[
                "--verify".into(),
                "--strict".into(),
                "-R".into(),
                format!("=anchor apple generic and certificate leaf[subject.OU] = \"{team}\""),
                bundle.display().to_string(),
            ],
            Path::new("/"),
            &[],
            Duration::from_secs(30),
            16384,
        )
        .map_err(|_| err("APP_SIGNATURE"))?;
    }
    Ok(())
}
fn running(destination: &Path) -> Result<(), AppError> {
    let output = process::run(
        Path::new("/bin/ps"),
        &["-axo".into(), "command=".into()],
        Path::new("/"),
        &[],
        Duration::from_secs(5),
        2 * 1024 * 1024,
    )?;
    if output.lines().any(|line| {
        line.trim()
            .starts_with(&format!("{}/Contents/", destination.display()))
    }) {
        return Err(AppError::new(
            "APP_RUNNING",
            "请先退出目标应用，再点击安装或升级。",
        ));
    }
    Ok(())
}
fn copy(source: &Path, destination: &Path) -> Result<(), AppError> {
    process::run(
        Path::new("/usr/bin/ditto"),
        &[
            source.display().to_string(),
            destination.display().to_string(),
        ],
        Path::new("/"),
        &[],
        Duration::from_secs(180),
        16384,
    )
    .map(|_| ())
}
fn unpack(
    package: &Path,
    directory: &Path,
    product: BundleProduct,
    zip: bool,
) -> Result<PathBuf, AppError> {
    let extracted = directory.join("extracted");
    files::private_directory(&extracted)?;
    if zip {
        artifacts::unpack(package, &extracted, true)?;
    } else {
        let mount = directory.join("mount");
        files::private_directory(&mount)?;
        process::run(
            Path::new("/usr/bin/hdiutil"),
            &[
                "attach".into(),
                package.display().to_string(),
                "-nobrowse".into(),
                "-readonly".into(),
                "-mountpoint".into(),
                mount.display().to_string(),
                "-quiet".into(),
            ],
            Path::new("/"),
            &[],
            Duration::from_secs(90),
            16384,
        )?;
        let copied = copy(&mount.join(product.name()), &extracted.join(product.name()));
        let detached = process::run(
            Path::new("/usr/bin/hdiutil"),
            &[
                "detach".into(),
                mount.display().to_string(),
                "-quiet".into(),
            ],
            Path::new("/"),
            &[],
            Duration::from_secs(60),
            16384,
        );
        copied?;
        detached?;
    }
    let bundle = extracted.join(product.name());
    if !bundle.is_dir() {
        return Err(err("APP_PACKAGE"));
    }
    Ok(bundle)
}
pub fn prepare(
    root: &Path,
    home: &Path,
    product: BundleProduct,
    package: &Package,
    arch: &str,
) -> Result<Record, AppError> {
    let destination = destination(home, product)?;
    prepare_at(root, home, product, package, arch, &destination)
}
/// A caller-controlled destination is permitted only inside the fixed Applications roots.
/// IPC always chooses a detected installation; this also supports isolated bundle tests.
pub fn prepare_at(
    root: &Path,
    home: &Path,
    product: BundleProduct,
    package: &Package,
    arch: &str,
    destination: &Path,
) -> Result<Record, AppError> {
    validate_destination(home, product, destination)?;
    let destination = destination.to_path_buf();

    if product != BundleProduct::Agentupdate {
        running(&destination)?;
    }
    let config = snapshot::scan(home, Some(product.config()))?;
    if config.bytes > 64 * 1024 * 1024 {
        return Err(err("APP_CONFIG_SIZE"));
    }
    let original = if destination.exists() {
        Some(snapshot::scan(&destination, None)?)
    } else {
        None
    };
    snapshot::require_space(
        snapshot::available_space(root)?,
        original.as_ref().map_or(0, |s| s.bytes).saturating_mul(2) + 4 * 1024 * 1024 * 1024,
    )?;
    files::private_directory(&root.join("app-installs"))?;
    let id = uuid::Uuid::new_v4().to_string();
    let work = work(root, &id)?;
    files::private_directory(&work)?;
    let path = work.join(if package.zip {
        "package.zip"
    } else {
        "package.dmg"
    });
    artifacts::download(&package.url, &package.sha256, &path)?;
    crate::progress::phase(crate::progress::Phase::Staging);
    let bundle = unpack(&path, &work, product, package.zip)?;
    verify_bundle(&bundle, product, &package.version, arch)?;
    let staged = snapshot::scan(&bundle, None)?;
    crate::progress::phase(crate::progress::Phase::BackingUp);
    snapshot::copy(home, &work.join("config"), &config)?;
    if let Some(old) = &original {
        copy(&destination, &work.join("original.app"))?;
        snapshot::verify_tree(&work.join("original.app"), old)?;
    }
    let r = Record {
        id,
        product,
        version: package.version.clone(),
        destination,
        state: "prepared".into(),
        failure_code: None,
        created_at: storage::now(),
        original,
        staged,
        config,
        home: home.into(),
    };
    save(&work, &r)?;
    Ok(r)
}
fn validate_record(root: &Path, home: &Path, id: &str) -> Result<(PathBuf, Record), AppError> {
    let work = work(root, id)?;
    let r = read(&work)?;
    if r.id != id || r.home != home {
        return Err(err("APP_INSTALL_ID"));
    }
    validate_destination(home, r.product, &r.destination)?;
    Ok((work, r))
}
pub fn install(root: &Path, home: &Path, id: &str, arch: &str) -> Result<Record, AppError> {
    let _lock = OperationLock::acquire(&root.join("locks/operation.lock"))?;
    install_locked(root, home, id, arch)
}
pub(crate) fn install_locked(
    root: &Path,
    home: &Path,
    id: &str,
    arch: &str,
) -> Result<Record, AppError> {
    let (work, mut r) = validate_record(root, home, id)?;
    if r.state != "prepared" {
        return Err(err("APP_INSTALL_STATE"));
    }
    running(&r.destination)?;
    let source = work.join("extracted").join(r.product.name());
    snapshot::verify_tree(&source, &r.staged)?;
    verify_bundle(&source, r.product, &r.version, arch)?;
    if let Some(old) = &r.original {
        snapshot::verify_tree(&r.destination, old)?;
        snapshot::verify_tree(&work.join("original.app"), old)?;
    } else if r.destination.exists() {
        return Err(err("APP_CHANGED"));
    }
    if snapshot::scan(home, Some(r.product.config()))?.entries != r.config.entries {
        return Err(err("APP_CONFIG_CHANGED"));
    }
    let replacement = r
        .destination
        .parent()
        .unwrap()
        .join(format!(".agentupdate-new-{}.app", r.id));
    let displaced = r
        .destination
        .parent()
        .unwrap()
        .join(format!(".agentupdate-old-{}.app", r.id));
    if replacement.exists() || displaced.exists() {
        return Err(err("APP_SWAP_CONFLICT"));
    }
    crate::progress::phase(crate::progress::Phase::Installing);
    copy(&source, &replacement)?;
    snapshot::verify_tree(&replacement, &r.staged)?;
    running(&r.destination)?;
    if let Some(old) = &r.original {
        snapshot::verify_tree(&r.destination, old)?;
    } else if fs::symlink_metadata(&r.destination).is_ok() {
        return Err(err("APP_CHANGED"));
    }
    if snapshot::scan(home, Some(r.product.config()))?.entries != r.config.entries {
        return Err(err("APP_CONFIG_CHANGED"));
    }
    r.state = "installing".into();
    save(&work, &r)?;
    let result: Result<(), AppError> = (|| {
        if r.destination.exists() {
            fs::rename(&r.destination, &displaced).map_err(|_| err("APP_SWAP"))?;
        }
        fs::rename(&replacement, &r.destination).map_err(|_| err("APP_SWAP"))?;
        files::sync_directory(r.destination.parent().unwrap())?;
        crate::progress::phase(crate::progress::Phase::Verifying);
        snapshot::verify_tree(&r.destination, &r.staged)?;
        verify_bundle(&r.destination, r.product, &r.version, arch)?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            r.state = "succeeded".into();
            save(&work, &r)?;
        }
        Err(e) => {
            r.state = "needs_recovery".into();
            r.failure_code = Some(e.code.into());
            save(&work, &r)?;
            let _ = restore(root, home, id);
            return Err(e);
        }
    }
    Ok(r)
}
pub fn rollback(root: &Path, home: &Path, id: &str) -> Result<Record, AppError> {
    let _lock = OperationLock::acquire(&root.join("locks/operation.lock"))?;
    restore(root, home, id)
}
fn restore(root: &Path, home: &Path, id: &str) -> Result<Record, AppError> {
    let (work, mut r) = validate_record(root, home, id)?;
    if ![
        "succeeded",
        "needs_recovery",
        "installing",
        "installed_not_restarted",
    ]
    .contains(&r.state.as_str())
    {
        return Err(err("APP_INSTALL_STATE"));
    }
    running(&r.destination)?;
    if r.destination.exists() {
        if let Some(old) = &r.original {
            if snapshot::verify_tree(&r.destination, old).is_ok() {
                r.state = "restored".into();
                save(&work, &r)?;
                return Ok(r);
            }
        }
        snapshot::verify_tree(&r.destination, &r.staged)?;
    }
    let rescue = r
        .destination
        .parent()
        .unwrap()
        .join(format!(".agentupdate-rescue-{}.app", r.id));
    if rescue.exists() {
        return Err(err("APP_SWAP_CONFLICT"));
    }
    if let Some(old) = &r.original {
        snapshot::verify_tree(&work.join("original.app"), old)?;
        let replacement = r
            .destination
            .parent()
            .unwrap()
            .join(format!(".agentupdate-restore-{}.app", r.id));
        if replacement.exists() {
            return Err(err("APP_SWAP_CONFLICT"));
        }
        copy(&work.join("original.app"), &replacement)?;
        snapshot::verify_tree(&replacement, old)?;
        if r.destination.exists() {
            fs::rename(&r.destination, &rescue).map_err(|_| err("APP_RESTORE"))?;
        }
        fs::rename(&replacement, &r.destination).map_err(|_| err("APP_RESTORE"))?;
        snapshot::verify_tree(&r.destination, old)?;
    } else if r.destination.exists() {
        fs::rename(&r.destination, &rescue).map_err(|_| err("APP_RESTORE"))?;
    }
    files::sync_directory(r.destination.parent().unwrap())?;
    r.state = "restored".into();
    save(&work, &r)?;
    Ok(r)
}
pub fn launch_helper(root: &Path, id: &str, restore: bool) -> Result<(), AppError> {
    let work = work(root, id)?;
    let r = read(&work)?;
    if r.product != BundleProduct::Agentupdate
        || (!restore && r.state != "prepared")
        || (restore
            && ![
                "succeeded",
                "needs_recovery",
                "installing",
                "installed_not_restarted",
            ]
            .contains(&r.state.as_str()))
    {
        return Err(err("APP_SELF_STATE"));
    }
    let helper = work.join("agentupdate-helper");
    fs::copy(
        std::env::current_exe().map_err(|_| err("APP_HELPER"))?,
        &helper,
    )
    .map_err(|_| err("APP_HELPER"))?;
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))
        .map_err(|_| err("APP_HELPER"))?;
    let mut cmd = std::process::Command::new(&helper);
    cmd.args([
        if restore {
            "--restore-helper"
        } else {
            "--install-helper"
        },
        &root.display().to_string(),
        id,
        &std::process::id().to_string(),
    ])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd.spawn().map_err(|_| err("APP_HELPER"))?;
    Ok(())
}
pub fn helper_entry() -> bool {
    let args: Vec<_> = std::env::args().collect();
    let restore = args.get(1).map(String::as_str) == Some("--restore-helper");
    if !restore && args.get(1).map(String::as_str) != Some("--install-helper") {
        return false;
    }
    let result = (|| -> Result<(), AppError> {
        if args.len() != 5 {
            return Err(err("APP_HELPER_ARGS"));
        }
        let root = PathBuf::from(&args[2]);
        let id = &args[3];
        let pid: i32 = args[4].parse().map_err(|_| err("APP_HELPER_ARGS"))?;
        if pid < 2 {
            return Err(err("APP_HELPER_ARGS"));
        }
        let work = work(&root, id)?;
        let mut r = read(&work)?;
        if r.product != BundleProduct::Agentupdate
            || (!restore && r.state != "prepared")
            || (restore
                && ![
                    "succeeded",
                    "needs_recovery",
                    "installing",
                    "installed_not_restarted",
                ]
                .contains(&r.state.as_str()))
        {
            return Err(err("APP_SELF_STATE"));
        }
        for _ in 0..200 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if unsafe { libc::kill(pid, 0) } == 0 {
            r.state = "helper_failed".into();
            r.failure_code = Some("APP_EXIT_TIMEOUT".into());
            save(&work, &r)?;
            return Err(err("APP_EXIT_TIMEOUT"));
        }
        let mut installed = if restore {
            rollback(&root, &r.home, id)?
        } else {
            install(&root, &r.home, id, std::env::consts::ARCH)?
        };
        if process::run(
            Path::new("/usr/bin/open"),
            &[installed.destination.display().to_string()],
            Path::new("/"),
            &[],
            Duration::from_secs(15),
            8192,
        )
        .is_err()
        {
            installed.state = "installed_not_restarted".into();
            installed.failure_code = Some("APP_RESTART".into());
            save(&work, &installed)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        if args.len() == 5 {
            if let Ok(work) = work(Path::new(&args[2]), &args[3]) {
                if let Ok(mut r) = read(&work) {
                    if r.state == "prepared" {
                        r.state = "helper_failed".into();
                    }
                    r.failure_code = Some(e.code.into());
                    let _ = save(&work, &r);
                }
            }
        }
    }
    true
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub product: BundleProduct,
    pub version: String,
    pub destination: PathBuf,
    pub state: String,
    pub has_backup: bool,
    pub failure_code: Option<String>,
    pub created_at: u64,
}
impl From<Record> for Summary {
    fn from(r: Record) -> Self {
        Self {
            id: r.id,
            product: r.product,
            version: r.version,
            destination: r.destination,
            state: r.state,
            has_backup: r.original.is_some(),
            failure_code: r.failure_code,
            created_at: r.created_at,
        }
    }
}
