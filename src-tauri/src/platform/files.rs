use crate::error::AppError;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
pub fn default_data_root() -> Result<PathBuf, AppError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::unsupported_platform());
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| AppError::new("HOME_UNAVAILABLE", "无法确认应用数据目录。"))?;
    Ok(home.join("Library/Application Support/AgentUpdate"))
}
pub fn private_directory(path: &Path) -> Result<(), AppError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(AppError::new(
                "UNSAFE_DATA_PATH",
                "应用数据目录不是当前用户的普通目录。",
            ));
        }
    } else {
        fs::create_dir_all(path)
            .map_err(|_| AppError::new("DATA_DIRECTORY", "无法创建应用数据目录。"))?;
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| AppError::new("DATA_PERMISSIONS", "无法限制数据目录权限。"))
}
pub fn regular_private_file(path: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| AppError::new("FILE_UNAVAILABLE", "数据文件不可读取。"))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(AppError::new(
            "UNSAFE_DATA_PATH",
            "数据文件的类型或所有权不正确。",
        ));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| AppError::new("DATA_PERMISSIONS", "无法限制数据文件权限。"))
}
pub fn sync_directory(path: &Path) -> Result<(), AppError> {
    fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| AppError::new("DATA_SYNC", "无法确认数据已写入磁盘。"))
}
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::new("DATA_PATH", "数据路径无效。"))?;
    private_directory(parent)?;
    if path.exists() || fs::symlink_metadata(path).is_ok() {
        regular_private_file(path)?;
    }
    let temporary = parent.join(format!(".write-{}", uuid::Uuid::new_v4()));
    let outcome = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)
            .map_err(|_| AppError::new("DATA_WRITE", "无法建立数据文件。"))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| AppError::new("DATA_WRITE", "数据写入未完成。"))?;
        fs::rename(&temporary, path)
            .map_err(|_| AppError::new("DATA_WRITE", "无法发布完整数据文件。"))?;
        sync_directory(parent)
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(temporary);
    }
    outcome
}
