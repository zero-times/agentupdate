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
    pub fn shared(path: &Path) -> Result<Self, AppError> {
        Self::open(path, libc::LOCK_SH | libc::LOCK_NB)
    }
    pub fn wait(path: &Path) -> Result<Self, AppError> {
        Self::open(path, libc::LOCK_EX)
    }
    pub fn resource(root: &Path, key: &str, wait: bool) -> Result<Self, AppError> {
        use sha2::{Digest, Sha256};
        let name = format!("resource-{:x}.lock", Sha256::digest(key.as_bytes()));
        let path = root.join("locks").join(name);
        if wait {
            Self::wait(&path)
        } else {
            Self::acquire(&path)
        }
    }
    pub fn npm_wait(prefix: &Path) -> Result<Self, AppError> {
        // Validate ownership and canonical identity before waiting on the same inode.
        match Self::npm(prefix) {
            Ok(lock) => Ok(lock),
            Err(e) if e.code == "OPERATION_BUSY" => {
                Self::wait(&prefix.join(".agentupdate-npm.lock"))
            }
            Err(e) => Err(e),
        }
    }
    pub fn acquire(path: &Path) -> Result<Self, AppError> {
        Self::open(path, libc::LOCK_EX | libc::LOCK_NB)
    }
    fn open(path: &Path, flags: i32) -> Result<Self, AppError> {
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
        if unsafe { libc::flock(file.as_raw_fd(), flags) } != 0 {
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
