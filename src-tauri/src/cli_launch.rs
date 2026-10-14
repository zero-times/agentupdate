use crate::{agents::discovery::Installation, error::AppError, platform};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

const LIMIT: usize = 20;
const TERMINAL_SCRIPT: &str = r#"on run argv
    tell application "Terminal"
        activate
        do script (item 1 of argv)
    end tell
end run"#;

pub fn projects(root: &Path) -> Result<Vec<String>, AppError> {
    let file = root.join("cli-projects.json");
    if !file.exists() {
        return Ok(Vec::new());
    }
    platform::files::regular_private_file(&file)?;
    let bytes = fs::read(&file)
        .map_err(|_| AppError::new("CLI_PROJECTS_READ", "无法读取最近项目目录。"))?;
    let paths: Vec<String> = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::new("CLI_PROJECTS_READ", "最近项目目录记录无法读取。"))?;
    if paths.len() > LIMIT || paths.iter().any(|p| !valid_path_text(p)) {
        return Err(AppError::new("CLI_PROJECTS_READ", "最近项目目录记录无效。"));
    }
    Ok(paths)
}

fn valid_path_text(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.chars().any(char::is_control)
        && Path::new(path).is_absolute()
}

pub fn project_directory(path: &str) -> Result<PathBuf, AppError> {
    if !valid_path_text(path) {
        return Err(AppError::new(
            "CLI_PROJECT_PATH",
            "请输入项目目录的绝对路径。",
        ));
    }
    let directory = fs::canonicalize(path)
        .map_err(|_| AppError::new("CLI_PROJECT_PATH", "项目目录不存在或无法访问，请修改路径。"))?;
    if !directory.is_dir() || directory.to_str().is_none_or(|p| !valid_path_text(p)) {
        return Err(AppError::new(
            "CLI_PROJECT_PATH",
            "请选择可访问的项目目录。",
        ));
    }
    fs::read_dir(&directory)
        .map_err(|_| AppError::new("CLI_PROJECT_PATH", "项目目录无法访问。"))?;
    Ok(directory)
}

pub fn save_project(
    root: &Path,
    path: &str,
    previous: Option<&str>,
) -> Result<Vec<String>, AppError> {
    let directory = project_directory(path)?;
    let path = directory.to_str().unwrap();
    let mut paths = projects(root)?;
    paths.retain(|p| p != path && Some(p.as_str()) != previous);
    paths.insert(0, path.to_owned());
    paths.truncate(LIMIT);
    platform::files::write_atomic(
        &root.join("cli-projects.json"),
        &serde_json::to_vec(&paths).unwrap(),
    )?;
    Ok(paths)
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn terminal_command(installation: &Installation, project: &Path) -> Result<String, AppError> {
    let resolved = fs::canonicalize(&installation.command_path)
        .map_err(|_| AppError::new("CLI_COMMAND_CHANGED", "CLI 命令已失效，请重新扫描。"))?;
    let metadata = fs::metadata(&resolved)
        .map_err(|_| AppError::new("CLI_COMMAND_CHANGED", "无法读取 CLI 命令，请重新扫描。"))?;
    if resolved != installation.resolved_command
        || !metadata.is_file()
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(AppError::new(
            "CLI_COMMAND_CHANGED",
            "CLI 命令已变化或不可执行，请重新扫描。",
        ));
    }
    let command = installation
        .command_path
        .to_str()
        .filter(|p| valid_path_text(p))
        .ok_or_else(|| AppError::new("CLI_COMMAND_PATH", "CLI 命令路径无效。"))?;
    let project = project
        .to_str()
        .filter(|p| valid_path_text(p))
        .ok_or_else(|| AppError::new("CLI_PROJECT_PATH", "项目目录路径无效。"))?;
    let mut directories = vec![installation.command_path.parent().unwrap().to_path_buf()];
    if let Some(node) = &installation.node_path {
        if let Some(parent) = node.parent() {
            directories.insert(0, parent.to_path_buf());
        }
    }
    // Shell and AppleScript have separate boundaries: quote shell arguments here;
    // pass the entire command to the fixed AppleScript as argv, never as source.
    let prefix = std::env::join_paths(directories)
        .map_err(|_| AppError::new("CLI_COMMAND_PATH", "CLI 环境路径无效。"))?;
    let prefix = prefix
        .to_str()
        .ok_or_else(|| AppError::new("CLI_COMMAND_PATH", "CLI 环境路径无效。"))?;
    Ok(format!(
        "cd -- {} && PATH={}:\"$PATH\" {}",
        quote(project),
        quote(prefix),
        quote(command)
    ))
}

pub fn launch(installation: &Installation, path: &str) -> Result<PathBuf, AppError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::unsupported_platform());
    }
    let directory = project_directory(path)?;
    let command = terminal_command(installation, &directory)?;
    platform::process::run(Path::new("/usr/bin/osascript"), &["-e".into(), TERMINAL_SCRIPT.into(), command], &directory,
        &[("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into())], Duration::from_secs(30), 4096)
        .map_err(|_| AppError::new("CLI_TERMINAL_OPEN", "无法打开终端。请在系统设置 → 隐私与安全性 → 自动化中允许 AgentUpdate 控制终端，然后重试。"))?;
    Ok(directory)
}
