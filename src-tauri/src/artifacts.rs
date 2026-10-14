//! Bounded downloads and extraction. Callers must validate the product's original URL.
use crate::{
    error::AppError,
    platform::{files, network, process},
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::Duration,
};

fn error() -> AppError {
    AppError::new(
        "ARTIFACT_DOWNLOAD",
        "可信制品下载或完整性验证失败，安装未修改。",
    )
}
pub fn redirect_allowed(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "downloads.claude.ai"
                    | "persistent.oaistatic.com"
            )
        )
}
pub fn response(url: &str, timeout: u64) -> Result<reqwest::blocking::Response, AppError> {
    let client = network::client(Duration::from_secs(15), Duration::from_secs(timeout))?;
    let mut url = reqwest::Url::parse(url).map_err(|_| error())?;
    for _ in 0..6 {
        let r = client
            .get(url.clone())
            .header("User-Agent", "AgentUpdate")
            .send()
            .map_err(|_| error())?;
        if r.status().is_success() {
            return Ok(r);
        }
        if !r.status().is_redirection() {
            return Err(error());
        }
        let location = r
            .headers()
            .get("location")
            .and_then(|s| s.to_str().ok())
            .ok_or_else(error)?;
        let next = url.join(location).map_err(|_| error())?;
        if !redirect_allowed(&next) {
            return Err(error());
        }
        url = next;
    }
    Err(error())
}
pub fn small_document(url: &str, limit: u64) -> Result<Vec<u8>, AppError> {
    let r = response(url, 30)?;
    if r.content_length().is_some_and(|n| n > limit) {
        return Err(error());
    }
    let mut bytes = vec![];
    r.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error())?;
    if bytes.len() as u64 > limit {
        return Err(error());
    }
    Ok(bytes)
}
pub fn sha256(path: &Path) -> Result<String, AppError> {
    let mut file = fs::File::open(path).map_err(|_| error())?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|_| error())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn download(url: &str, digest: &str, path: &Path) -> Result<(), AppError> {
    crate::progress::phase(crate::progress::Phase::Downloading);
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error());
    }
    if path.exists() {
        files::regular_private_file(path)?;
        if sha256(path)? == digest.to_lowercase() {
            return Ok(());
        }
        return Err(error());
    }
    let (r, total) = download_reader(url, digest)?;
    const LIMIT: u64 = 1024 * 1024 * 1024;
    if total.is_some_and(|n| n > LIMIT) {
        return Err(error());
    }
    let partial = path.with_extension("partial");
    files::write_atomic(&partial, b"")?;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(&partial)
            .map_err(|_| error())?;
        let size =
            crate::progress::copy_download(r, &mut file, LIMIT, total).map_err(|_| error())?;
        file.flush().map_err(|_| error())?;
        file.sync_all().map_err(|_| error())?;
        if size > LIMIT || sha256(&partial)? != digest.to_lowercase() {
            return Err(error());
        }
        fs::rename(&partial, path).map_err(|_| error())?;
        files::sync_directory(path.parent().unwrap())
    })();
    if result.is_err() {
        let _ = fs::remove_file(partial);
    }
    result
}
fn download_reader(url: &str, _digest: &str) -> Result<(Box<dyn Read>, Option<u64>), AppError> {
    #[cfg(feature = "desktop-validation")]
    if let Some(root) = std::env::var_os("AGENTUPDATE_TEST_DESKTOP_DIR") {
        struct Fixture(fs::File);
        impl Read for Fixture {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                std::thread::sleep(Duration::from_millis(20));
                self.0.read(bytes)
            }
        }
        let file = fs::File::open(std::path::PathBuf::from(root).join(format!("{_digest}.bin")))
            .map_err(|_| error())?;
        let size = file.metadata().map_err(|_| error())?.len();
        return Ok((Box::new(Fixture(file)), Some(size)));
    }
    let response = response(url, 600)?;
    let total = response.content_length();
    Ok((Box::new(response), total))
}
pub fn unpack(package: &Path, destination: &Path, zip: bool) -> Result<(), AppError> {
    files::private_directory(destination)?;
    let args = if zip {
        vec!["-Z1".into(), package.display().to_string()]
    } else {
        vec!["-tzf".into(), package.display().to_string()]
    };
    let list = process::run(
        Path::new(if zip {
            "/usr/bin/unzip"
        } else {
            "/usr/bin/tar"
        }),
        &args,
        Path::new("/"),
        &[],
        Duration::from_secs(30),
        8 * 1024 * 1024,
    )?;
    if list
        .lines()
        .any(|n| n.starts_with('/') || n.split('/').any(|p| p == ".."))
    {
        return Err(AppError::new("ARCHIVE_PATH", "安装包含有越界路径。"));
    }
    let profile = format!(
        "(version 1)(allow default)(deny file-write*)(allow file-write* (subpath {}))",
        serde_json::to_string(&destination.to_string_lossy()).unwrap()
    );
    let mut args = vec!["-p".into(), profile];
    if zip {
        args.extend([
            "/usr/bin/ditto".into(),
            "-x".into(),
            "-k".into(),
            package.display().to_string(),
            destination.display().to_string(),
        ]);
    } else {
        args.extend([
            "/usr/bin/tar".into(),
            "-xzf".into(),
            package.display().to_string(),
            "-C".into(),
            destination.display().to_string(),
        ]);
    }
    process::run(
        Path::new("/usr/bin/sandbox-exec"),
        &args,
        Path::new("/"),
        &[],
        Duration::from_secs(120),
        1024 * 1024,
    )
    .map(|_| ())
}
