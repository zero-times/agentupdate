use crate::{error::AppError, platform::files};
use std::{
    fs::{File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
};
/// Stable lock inode: never remove a lock file, which could create two lock domains.
pub struct OperationLock(File);
impl OperationLock {
    pub fn npm(prefix: &Path) -> Result<Self, AppError> {
        if std::fs::canonicalize(prefix).ok().as_deref() != Some(prefix) {
            return Err(AppError::new(
                "INSTALLATION_CHANGED",
                "安装目录已变化，请重新扫描。",
            ));
        }
        let m = std::fs::symlink_metadata(prefix)
            .map_err(|_| AppError::new("LOCK_UNAVAILABLE", "安装目录无法读取。"))?;
        if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } {
            return Err(AppError::new("LOCK_UNSAFE", "安装目录所有权或类型已变化。"));
        }
        Self::acquire(&prefix.join(".agentupdate-npm.lock"))
    }
    pub fn acquire(path: &Path) -> Result<Self, AppError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| AppError::new("LOCK_UNAVAILABLE", "无法建立操作锁，请检查目录权限。"))?;
        let m = file
            .metadata()
            .map_err(|_| AppError::new("LOCK_UNAVAILABLE", "无法核验操作锁。"))?;
        if !m.is_file() || m.uid() != unsafe { libc::geteuid() } || m.nlink() != 1 {
            return Err(AppError::new("LOCK_UNSAFE", "操作锁的类型或所有权不正确。"));
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(AppError::new(
                "OPERATION_BUSY",
                "另一进程正在更新或恢复，等待其结束后重试。",
            ));
        }
        files::regular_private_file(path)?;
        Ok(Self(file))
    }
}
impl Drop for OperationLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
