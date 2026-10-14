use crate::{error::AppError, platform::files};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};
const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub path: String,
    pub kind: String,
    pub mode: u32,
    pub size: u64,
    pub sha256: Option<String>,
    pub link: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub entries: Vec<Entry>,
    pub bytes: u64,
}
fn fail(code: &'static str) -> AppError {
    AppError::new(code, "备份内容、权限或路径无法可靠验证；未修改安装。")
}
pub fn relative(value: &str) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || value.is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(fail("UNSAFE_SNAPSHOT_PATH"));
    }
    Ok(path)
}
fn controlled_link(root: &Path, path: &Path, target: &Path) -> Result<PathBuf, AppError> {
    if target.is_absolute() {
        return Err(fail("EXTERNAL_SYMLINK"));
    }
    let resolved = fs::canonicalize(path.parent().unwrap().join(target))
        .map_err(|_| fail("BROKEN_SYMLINK"))?;
    let base = fs::canonicalize(root).map_err(|_| fail("SNAPSHOT_SOURCE"))?;
    if !resolved.starts_with(&base) {
        return Err(fail("EXTERNAL_SYMLINK"));
    }
    Ok(resolved.strip_prefix(base).unwrap().to_path_buf())
}
fn read_file(path: &Path) -> Result<(String, u64), AppError> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| fail("BACKUP_READ"))?;
    let metadata = file.metadata().map_err(|_| fail("BACKUP_READ"))?;
    if !metadata.is_file()
        || metadata.len() > MAX_BYTES
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o400 == 0
    {
        return Err(fail("BACKUP_PERMISSIONS"));
    }
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut reader = file.take(MAX_BYTES + 1);
    let mut buffer = [0u8; 65536];
    loop {
        let n = reader.read(&mut buffer).map_err(|_| fail("BACKUP_READ"))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    if size != metadata.len() {
        return Err(fail("BACKUP_CHANGED"));
    }
    Ok((format!("{:x}", hash.finalize()), size))
}
pub fn scan(root: &Path, selected: Option<&[&str]>) -> Result<Snapshot, AppError> {
    let mut result = Snapshot {
        entries: Vec::new(),
        bytes: 0,
    };
    let mut seen = HashSet::new();
    let mut pending: Vec<PathBuf> = selected
        .map(|s| s.iter().map(PathBuf::from).collect())
        .unwrap_or_else(|| vec![PathBuf::from(".")]);
    while let Some(relative_path) = pending.pop() {
        let name = relative_path
            .to_str()
            .ok_or_else(|| fail("SNAPSHOT_PATH_ENCODING"))?
            .to_string();
        relative(&name)?;
        if !seen.insert(name.clone()) {
            continue;
        }
        if seen.len() > MAX_ENTRIES {
            return Err(fail("SNAPSHOT_LIMIT"));
        }
        ensure_parents(root, &relative_path)?;
        let path = root.join(&relative_path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && selected.is_some() => {
                result.entries.push(Entry {
                    path: name,
                    kind: "missing".into(),
                    mode: 0,
                    size: 0,
                    sha256: None,
                    link: None,
                });
                continue;
            }
            Err(_) => return Err(fail("BACKUP_READ")),
        };
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(fail("BACKUP_PERMISSIONS"));
        }
        let mut entry = Entry {
            path: name,
            kind: String::new(),
            mode: metadata.mode() & 0o777,
            size: 0,
            sha256: None,
            link: None,
        };
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path).map_err(|_| fail("BACKUP_READ"))?;
            let resolved = controlled_link(root, &path, &target)?;
            if selected.is_some() {
                pending.push(resolved);
            }
            entry.kind = "link".into();
            entry.link = Some(
                target
                    .to_str()
                    .ok_or_else(|| fail("SNAPSHOT_PATH_ENCODING"))?
                    .into(),
            );
        } else if metadata.is_dir() {
            if metadata.mode() & 0o500 != 0o500 {
                return Err(fail("BACKUP_PERMISSIONS"));
            }
            entry.kind = "directory".into();
            for child in fs::read_dir(&path).map_err(|_| fail("BACKUP_READ"))? {
                pending
                    .push(relative_path.join(child.map_err(|_| fail("BACKUP_READ"))?.file_name()));
            }
        } else if metadata.is_file() {
            let (hash, size) = read_file(&path)?;
            entry.kind = "file".into();
            entry.size = size;
            entry.sha256 = Some(hash);
            result.bytes = result
                .bytes
                .checked_add(size)
                .ok_or_else(|| fail("SNAPSHOT_LIMIT"))?;
            if result.bytes > MAX_BYTES {
                return Err(fail("SNAPSHOT_LIMIT"));
            }
        } else {
            return Err(fail("UNSUPPORTED_FILE_TYPE"));
        }
        result.entries.push(entry);
    }
    if selected.is_some() {
        // Record only managed leaves' ancestors, without capturing unrelated siblings.
        let leaves = result.entries.clone();
        for leaf in leaves {
            let path = relative(&leaf.path)?;
            let mut parent = path.parent();
            while let Some(p) = parent.filter(|p| !p.as_os_str().is_empty() && *p != Path::new("."))
            {
                let name = p
                    .to_str()
                    .ok_or_else(|| fail("SNAPSHOT_PATH_ENCODING"))?
                    .to_string();
                if seen.insert(name.clone()) {
                    let m = match fs::symlink_metadata(root.join(p)) {
                        Ok(m) => m,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            parent = p.parent();
                            continue;
                        }
                        Err(_) => return Err(fail("BACKUP_READ")),
                    };
                    if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } {
                        return Err(fail("BACKUP_PERMISSIONS"));
                    }
                    result.entries.push(Entry {
                        path: name,
                        kind: "directory".into(),
                        mode: m.mode() & 0o777,
                        size: 0,
                        sha256: None,
                        link: None,
                    });
                }
                parent = p.parent();
            }
        }
    }
    result.entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}
pub fn ensure_parents(root: &Path, relative_path: &Path) -> Result<(), AppError> {
    // Never follow an existing symlink in an intermediate source/destination component.
    let mut current = root.to_path_buf();
    if let Ok(m) = fs::symlink_metadata(root) {
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err(fail("UNSAFE_SNAPSHOT_PATH"));
        }
    }
    for component in relative_path.parent().unwrap_or(Path::new("")).components() {
        current.push(component.as_os_str());
        if let Ok(m) = fs::symlink_metadata(&current) {
            if !m.is_dir() || m.file_type().is_symlink() {
                return Err(fail("UNSAFE_SNAPSHOT_PATH"));
            }
        }
    }
    Ok(())
}
fn data_name(entry: &Entry) -> String {
    format!("{:x}", Sha256::digest(entry.path.as_bytes()))
}
pub fn copy(root: &Path, destination: &Path, snapshot: &Snapshot) -> Result<(), AppError> {
    files::private_directory(destination)?;
    for entry in &snapshot.entries {
        if entry.kind != "file" {
            continue;
        }
        let path = relative(&entry.path)?;
        ensure_parents(root, &path)?;
        let mut source = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(root.join(path))
            .map_err(|_| fail("BACKUP_READ"))?;
        let target = destination.join(data_name(entry));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(target)
            .map_err(|_| fail("BACKUP_WRITE"))?;
        let n = std::io::copy(&mut (&mut source).take(entry.size + 1), &mut file)
            .map_err(|_| fail("BACKUP_WRITE"))?;
        if n != entry.size {
            return Err(fail("BACKUP_CHANGED"));
        }
        file.flush()
            .and_then(|_| file.sync_all())
            .map_err(|_| fail("DATA_SYNC"))?;
    }
    files::sync_directory(destination)?;
    verify(destination, snapshot)?;
    // Recheck the source after copying: a changed file or link aborts publication.
    for entry in &snapshot.entries {
        let path = root.join(relative(&entry.path)?);
        match entry.kind.as_str() {
            "file" => {
                if read_file(&path)? != (entry.sha256.clone().unwrap(), entry.size) {
                    return Err(fail("BACKUP_CHANGED"));
                }
            }
            "link" => {
                if fs::read_link(path)
                    .ok()
                    .and_then(|p| p.to_str().map(String::from))
                    != entry.link
                {
                    return Err(fail("BACKUP_CHANGED"));
                }
            }
            "missing" => {
                if fs::symlink_metadata(path).is_ok() {
                    return Err(fail("BACKUP_CHANGED"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}
pub fn verify(directory: &Path, snapshot: &Snapshot) -> Result<(), AppError> {
    let mut seen = HashSet::new();
    let mut bytes = 0u64;
    for entry in &snapshot.entries {
        relative(&entry.path)?;
        if !seen.insert(&entry.path) || entry.mode & !0o777 != 0 {
            return Err(fail("SNAPSHOT_CORRUPT"));
        }
        match entry.kind.as_str() {
            "file" => {
                let path = directory.join(data_name(entry));
                files::regular_private_file(&path)?;
                let (hash, size) = read_file(&path)?;
                if Some(hash) != entry.sha256 || size != entry.size {
                    return Err(fail("SNAPSHOT_CORRUPT"));
                }
                bytes += size;
            }
            "directory" | "missing" => {}
            "link" => {
                if entry
                    .link
                    .as_ref()
                    .is_none_or(|l| Path::new(l).is_absolute())
                {
                    return Err(fail("SNAPSHOT_CORRUPT"));
                }
            }
            _ => return Err(fail("SNAPSHOT_CORRUPT")),
        }
    }
    if bytes != snapshot.bytes || bytes > MAX_BYTES || snapshot.entries.len() > MAX_ENTRIES {
        return Err(fail("SNAPSHOT_CORRUPT"));
    }
    Ok(())
}
pub fn materialize(
    directory: &Path,
    snapshot: &Snapshot,
    destination: &Path,
) -> Result<(), AppError> {
    verify(directory, snapshot)?;
    files::private_directory(destination)?;
    for entry in &snapshot.entries {
        if entry.kind == "directory" {
            let p = destination.join(relative(&entry.path)?);
            ensure_parents(destination, &relative(&entry.path)?)?;
            fs::create_dir_all(&p).map_err(|_| fail("RESTORE_WRITE"))?;
        }
    }
    for entry in &snapshot.entries {
        let rel = relative(&entry.path)?;
        ensure_parents(destination, &rel)?;
        let p = destination.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(|_| fail("RESTORE_WRITE"))?;
        }
        match entry.kind.as_str() {
            "file" => {
                fs::copy(directory.join(data_name(entry)), &p)
                    .map_err(|_| fail("RESTORE_WRITE"))?;
                fs::set_permissions(&p, fs::Permissions::from_mode(entry.mode))
                    .map_err(|_| fail("RESTORE_PERMISSIONS"))?;
                fs::File::open(&p)
                    .and_then(|f| f.sync_all())
                    .map_err(|_| fail("DATA_SYNC"))?;
            }
            "link" => {
                std::os::unix::fs::symlink(entry.link.as_ref().unwrap(), &p)
                    .map_err(|_| fail("RESTORE_WRITE"))?;
            }
            "directory" | "missing" => {}
            _ => return Err(fail("SNAPSHOT_CORRUPT")),
        }
    }
    for entry in snapshot.entries.iter().rev() {
        if entry.kind == "directory" {
            let p = destination.join(relative(&entry.path)?);
            fs::set_permissions(&p, fs::Permissions::from_mode(entry.mode))
                .map_err(|_| fail("RESTORE_PERMISSIONS"))?;
            files::sync_directory(&p)?;
        }
    }
    for entry in &snapshot.entries {
        if entry.kind == "link" {
            controlled_link(
                destination,
                &destination.join(relative(&entry.path)?),
                Path::new(entry.link.as_ref().unwrap()),
            )?;
        }
    }
    files::sync_directory(destination)
}
pub fn available_space(path: &Path) -> Result<u64, AppError> {
    use std::ffi::CString;
    let c = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_| fail("DISK_SPACE"))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(c.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(fail("DISK_SPACE"));
    }
    let stat = unsafe { stat.assume_init() };
    Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}
pub fn require_space(available: u64, required: u64) -> Result<(), AppError> {
    if available < required {
        return Err(AppError::new(
            "DISK_FULL",
            "磁盘空间不足；保留现有恢复点，停止本次更新。",
        ));
    }
    Ok(())
}

/// Verify a materialized installation tree (archive verification uses hashed payload names).
pub fn verify_tree(root: &Path, expected: &Snapshot) -> Result<(), AppError> {
    let current = scan(root, None)?;
    if current.bytes != expected.bytes || current.entries != expected.entries {
        return Err(fail("INSTALLATION_CHANGED"));
    }
    Ok(())
}
