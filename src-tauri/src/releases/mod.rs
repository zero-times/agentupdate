use crate::{
    error::AppError,
    platform::{files, locks::OperationLock},
    storage,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path, time::Duration};
pub const SOURCE: &str = "https://github.com/zero-times/agentupdate/releases";
const API: &str = "https://api.github.com/repos/zero-times/agentupdate/releases";
const DAY: u64 = 24 * 60 * 60 * 1000;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseStatus {
    pub state: &'static str,
    pub source: &'static str,
}
pub fn status() -> ReleaseStatus {
    ReleaseStatus {
        state: "not_checked",
        source: SOURCE,
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    pub version: String,
    pub tag: String,
    pub published_at: String,
    pub notes: String,
    pub url: String,
    pub assets: Vec<Asset>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cache {
    pub schema_version: u32,
    pub checked_at: Option<u64>,
    pub successful_at: Option<u64>,
    pub latest: Option<ReleaseInfo>,
    pub etag: Option<String>,
    pub error_code: Option<String>,
    pub retry_after: u64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub state: String,
    pub current_version: String,
    pub latest: Option<ReleaseInfo>,
    pub asset: Option<Asset>,
    pub checked_at: Option<u64>,
    pub successful_at: Option<u64>,
    pub cached: bool,
    pub error_code: Option<String>,
    pub retry_after: u64,
}
#[derive(Debug)]
pub struct FetchFailure {
    pub code: &'static str,
    pub retry_after: u64,
}
#[derive(Debug)]
pub enum FetchResult {
    Published(Option<ReleaseInfo>, Option<String>),
    NotModified,
}
fn response_error() -> AppError {
    AppError::new(
        "RELEASE_RESPONSE",
        "GitHub 发布响应无效，不能确认更新状态。",
    )
}
fn parse_tag(tag: &str) -> Option<Version> {
    let v = Version::parse(tag.strip_prefix('v')?).ok()?;
    if !v.pre.is_empty() || !v.build.is_empty() || tag != format!("v{v}") {
        None
    } else {
        Some(v)
    }
}
pub fn parse(json: &serde_json::Value) -> Result<Option<ReleaseInfo>, AppError> {
    let entries = json.as_array().ok_or_else(response_error)?;
    let mut latest: Option<(Version, ReleaseInfo)> = None;
    for entry in entries {
        let object = entry.as_object().ok_or_else(response_error)?;
        let draft = object
            .get("draft")
            .and_then(|v| v.as_bool())
            .ok_or_else(response_error)?;
        let pre = object
            .get("prerelease")
            .and_then(|v| v.as_bool())
            .ok_or_else(response_error)?;
        if draft || pre {
            continue;
        }
        let tag = object
            .get("tag_name")
            .and_then(|v| v.as_str())
            .ok_or_else(response_error)?;
        let Some(version) = parse_tag(tag) else {
            continue;
        };
        let url = format!("{SOURCE}/tag/{tag}");
        if object.get("html_url").and_then(|v| v.as_str()) != Some(&url) {
            return Err(response_error());
        }
        let published = object
            .get("published_at")
            .and_then(|v| v.as_str())
            .ok_or_else(response_error)?;
        if published.len() != 20 || !published.ends_with('Z') {
            return Err(response_error());
        }
        let notes = object.get("body").and_then(|v| v.as_str()).unwrap_or("");
        if notes.len() > 128 * 1024 {
            return Err(response_error());
        }
        let values = object
            .get("assets")
            .and_then(|v| v.as_array())
            .ok_or_else(response_error)?;
        let mut assets = vec![];
        for asset in values {
            let Some(name) = asset.get("name").and_then(|v| v.as_str()) else {
                return Err(response_error());
            };
            let expected = format!("{SOURCE}/download/{tag}/{name}");
            let valid_name = ["aarch64", "x86_64", "universal"]
                .iter()
                .any(|arch| name == format!("AgentUpdate-{}-macos-{arch}.dmg", version));
            if valid_name
                && asset.get("browser_download_url").and_then(|v| v.as_str()) == Some(&expected)
                && asset.get("state").and_then(|v| v.as_str()) == Some("uploaded")
            {
                assets.push(Asset {
                    name: name.into(),
                    url: expected,
                    sha256: asset["digest"]
                        .as_str()
                        .and_then(|s| s.strip_prefix("sha256:"))
                        .filter(|s| valid_digest(s))
                        .map(String::from),
                });
            }
        }
        let info = ReleaseInfo {
            version: version.to_string(),
            tag: tag.into(),
            published_at: published.into(),
            notes: notes.into(),
            url,
            assets,
        };
        if latest
            .as_ref()
            .is_none_or(|(current, _)| version > *current)
        {
            latest = Some((version, info));
        }
    }
    Ok(latest.map(|(_, info)| info))
}
pub fn evaluate(cache: &Cache, current: &str, arch: &str, cached: bool) -> CheckResult {
    let state = if cache.error_code.is_some() {
        "failed"
    } else if cache.checked_at.is_none() {
        "not_checked"
    } else {
        match &cache.latest {
            None => "no_releases",
            Some(latest) => match (Version::parse(current), Version::parse(&latest.version)) {
                (Ok(current), Ok(remote)) => match remote.cmp(&current) {
                    std::cmp::Ordering::Greater => "update_available",
                    std::cmp::Ordering::Equal => "up_to_date",
                    std::cmp::Ordering::Less => "local_newer",
                },
                _ => "failed",
            },
        }
    };
    let asset = cache.latest.as_ref().and_then(|r| {
        let arch = match arch {
            "aarch64" | "arm64" => "aarch64",
            "x86_64" => "x86_64",
            _ => return None,
        };
        [arch, "universal"].iter().find_map(|a| {
            r.assets
                .iter()
                .find(|asset| asset.name == format!("AgentUpdate-{}-macos-{a}.dmg", r.version))
                .cloned()
        })
    });
    CheckResult {
        state: state.into(),
        current_version: current.into(),
        latest: cache.latest.clone(),
        asset,
        checked_at: cache.checked_at,
        successful_at: cache.successful_at,
        cached,
        error_code: cache.error_code.clone(),
        retry_after: cache.retry_after,
    }
}
pub fn should_fetch(cache: &Cache, manual: bool, now: u64) -> bool {
    (manual && cache.error_code.as_deref() != Some("RELEASE_RATE_LIMIT")
        || now >= cache.retry_after)
        && (manual
            || cache.error_code.is_some()
            || cache
                .checked_at
                .is_none_or(|t| now < t || now.saturating_sub(t) >= DAY))
}
pub fn apply(cache: &mut Cache, result: Result<FetchResult, FetchFailure>, now: u64) {
    cache.schema_version = 1;
    cache.checked_at = Some(now);
    match result {
        Ok(FetchResult::Published(latest, etag)) => {
            cache.latest = latest;
            cache.etag = etag;
            cache.successful_at = Some(now);
            cache.error_code = None;
            cache.retry_after = 0;
        }
        Ok(FetchResult::NotModified) if cache.successful_at.is_some() => {
            cache.successful_at = Some(now);
            cache.error_code = None;
            cache.retry_after = 0;
        }
        Ok(FetchResult::NotModified) => {
            cache.error_code = Some("RELEASE_RESPONSE".into());
            cache.retry_after = now + 60_000;
        }
        Err(e) => {
            cache.error_code = Some(e.code.into());
            cache.retry_after = e.retry_after.max(now + 60_000);
        }
    }
}
fn failure(code: &'static str, retry_after: u64) -> FetchFailure {
    FetchFailure { code, retry_after }
}
pub fn fetch(cache: &Cache) -> Result<FetchResult, FetchFailure> {
    let client = crate::platform::network::client(Duration::from_secs(8), Duration::from_secs(12))
        .map_err(|_| failure("RELEASE_NETWORK", 0))?;
    fetch_with_client(cache, &client)
}
fn fetch_with_client(
    cache: &Cache,
    client: &reqwest::blocking::Client,
) -> Result<FetchResult, FetchFailure> {
    let mut latest: Option<ReleaseInfo> = None;
    let mut etag = None;
    for page in 1..=5 {
        let url = format!("{API}?per_page=100&page={page}");
        let mut response = None;
        for attempt in 0..2 {
            let mut request = client
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header(
                    "User-Agent",
                    concat!("AgentUpdate/", env!("CARGO_PKG_VERSION")),
                );
            if page == 1 {
                if let Some(tag) = cache
                    .etag
                    .as_ref()
                    .filter(|_| cache.successful_at.is_some())
                {
                    request = request.header("If-None-Match", tag);
                }
            }
            match request.send() {
                Ok(r) if r.status().as_u16() == 304 && page == 1 => {
                    return Ok(FetchResult::NotModified)
                }
                Ok(r) if r.status().is_success() => {
                    response = Some(r);
                    break;
                }
                Ok(r)
                    if r.status().as_u16() == 429
                        || (r.status().as_u16() == 403
                            && r.headers()
                                .get("x-ratelimit-remaining")
                                .is_some_and(|v| v == "0")) =>
                {
                    let now = storage::now();
                    let wait = r
                        .headers()
                        .get("retry-after")
                        .and_then(|s| s.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(|s| now.saturating_add(s.saturating_mul(1000)));
                    let reset = r
                        .headers()
                        .get("x-ratelimit-reset")
                        .and_then(|s| s.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(|s| s.saturating_mul(1000));
                    return Err(failure(
                        "RELEASE_RATE_LIMIT",
                        wait.or(reset)
                            .unwrap_or(now + 60_000)
                            .clamp(now + 60_000, now + DAY),
                    ));
                }
                Ok(r) if r.status().is_server_error() && attempt == 0 => {}
                Ok(r) => {
                    return Err(failure(
                        if r.status().as_u16() == 403 || r.status().as_u16() == 401 {
                            "RELEASE_PERMISSION"
                        } else {
                            "RELEASE_HTTP"
                        },
                        0,
                    ))
                }
                Err(_) if attempt == 0 => {}
                Err(_) => return Err(failure("RELEASE_NETWORK", 0)),
            }
            std::thread::sleep(Duration::from_millis(400));
        }
        let r = response.ok_or_else(|| failure("RELEASE_NETWORK", 0))?;
        if page == 1 {
            etag = r
                .headers()
                .get("etag")
                .and_then(|s| s.to_str().ok())
                .filter(|s| s.len() < 1024)
                .map(String::from);
        }
        let next = r
            .headers()
            .get("link")
            .and_then(|s| s.to_str().ok())
            .is_some_and(|s| s.contains("rel=\"next\""));
        const LIMIT: u64 = 4 * 1024 * 1024;
        if r.content_length().is_some_and(|n| n > LIMIT) {
            return Err(failure("RELEASE_RESPONSE", 0));
        }
        let mut bytes = vec![];
        r.take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failure("RELEASE_NETWORK", 0))?;
        if bytes.len() as u64 > LIMIT {
            return Err(failure("RELEASE_RESPONSE", 0));
        }
        let json = serde_json::from_slice(&bytes).map_err(|_| failure("RELEASE_RESPONSE", 0))?;
        let found = parse(&json).map_err(|_| failure("RELEASE_RESPONSE", 0))?;
        if let Some(found) = found {
            if latest.as_ref().is_none_or(|old| {
                Version::parse(&found.version).unwrap() > Version::parse(&old.version).unwrap()
            }) {
                latest = Some(found);
            }
        }
        if next {
            etag = None;
        }
        if !next {
            return Ok(FetchResult::Published(latest, etag));
        }
    }
    Err(failure("RELEASE_RESPONSE_LIMIT", 0))
}
fn read_cache(root: &Path) -> Result<Cache, AppError> {
    let path = root.join("release-cache.json");
    if fs::symlink_metadata(&path).is_err() {
        return Ok(Cache {
            schema_version: 1,
            ..Cache::default()
        });
    }
    files::regular_private_file(&path)?;
    let mut bytes = vec![];
    fs::File::open(path)
        .and_then(|f| f.take(1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|_| AppError::new("RELEASE_CACHE", "无法读取版本检查缓存。"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(AppError::new(
            "RELEASE_CACHE",
            "版本检查缓存损坏，原文件已保留。",
        ));
    }
    let cache: Cache = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::new("RELEASE_CACHE", "版本检查缓存损坏，原文件已保留。"))?;
    if cache.schema_version != 1 {
        return Err(AppError::new(
            "RELEASE_CACHE",
            "版本检查缓存来自不兼容版本。",
        ));
    }
    if let Some(info) = &cache.latest {
        if parse_tag(&info.tag).map(|v| v.to_string()).as_deref() != Some(&info.version)
            || info.url != format!("{SOURCE}/tag/{}", info.tag)
            || info.notes.len() > 128 * 1024
        {
            return Err(AppError::new(
                "RELEASE_CACHE",
                "版本检查缓存身份无效，原文件已保留。",
            ));
        }
        for asset in &info.assets {
            if !["aarch64", "x86_64", "universal"]
                .iter()
                .any(|arch| asset.name == format!("AgentUpdate-{}-macos-{arch}.dmg", info.version))
                || asset.url != format!("{SOURCE}/download/{}/{}", info.tag, asset.name)
                || asset.sha256.as_deref().is_some_and(|s| !valid_digest(s))
            {
                return Err(AppError::new(
                    "RELEASE_CACHE",
                    "版本检查缓存制品无效，原文件已保留。",
                ));
            }
        }
    }
    Ok(cache)
}
pub fn check(
    root: &Path,
    manual: bool,
    current: &str,
    arch: &str,
) -> Result<CheckResult, AppError> {
    let _lock = OperationLock::acquire(&root.join("locks/releases.lock"))?;
    let mut cache = read_cache(root)?;
    let now = storage::now();
    if !should_fetch(&cache, manual, now) {
        return Ok(evaluate(&cache, current, arch, true));
    }
    let result = match fetch(&cache) {
        Err(e)
            if [
                "RELEASE_RATE_LIMIT",
                "RELEASE_NETWORK",
                "RELEASE_HTTP",
                "RELEASE_PERMISSION",
            ]
            .contains(&e.code) =>
        {
            fallback().or(Err(e))
        }
        other => other,
    };
    apply(&mut cache, result, storage::now());
    files::write_atomic(
        &root.join("release-cache.json"),
        &serde_json::to_vec(&cache)
            .map_err(|_| AppError::new("RELEASE_CACHE", "无法保存版本检查缓存。"))?,
    )?;
    Ok(evaluate(&cache, current, arch, false))
}
pub fn open_link(root: &Path, asset: bool, current: &str, arch: &str) -> Result<(), AppError> {
    let cache = read_cache(root)?;
    let result = evaluate(&cache, current, arch, true);
    let latest = result
        .latest
        .as_ref()
        .ok_or_else(|| AppError::new("RELEASE_NOT_FOUND", "没有可打开的发布版本。"))?;
    let expected = format!("{SOURCE}/tag/{}", latest.tag);
    if parse_tag(&latest.tag).is_none() || latest.url != expected {
        return Err(response_error());
    }
    let url = if asset {
        let asset = result
            .asset
            .ok_or_else(|| AppError::new("RELEASE_ASSET_MISSING", "没有匹配当前架构的安装包。"))?;
        let expected = format!("{SOURCE}/download/{}/{}", latest.tag, asset.name);
        if asset.url != expected {
            return Err(response_error());
        }
        asset.url
    } else {
        latest.url.clone()
    };
    let status = std::process::Command::new("/usr/bin/open")
        .arg(&url)
        .status()
        .map_err(|_| AppError::new("RELEASE_OPEN", "无法打开发布页面。"))?;
    if !status.success() {
        return Err(AppError::new("RELEASE_OPEN", "无法打开发布页面。"));
    }
    Ok(())
}
#[cfg(test)]
mod network_tests {
    use super::*;
    #[test]
    fn offline_is_failure_after_bounded_retry() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let client = reqwest::blocking::Client::builder()
            .proxy(reqwest::Proxy::all(format!("http://{address}")).unwrap())
            .connect_timeout(Duration::from_millis(50))
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        let start = std::time::Instant::now();
        assert_eq!(
            fetch_with_client(&Cache::default(), &client)
                .unwrap_err()
                .code,
            "RELEASE_NETWORK"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn parse_checksums(bytes: &[u8], tag: &str) -> Result<Vec<Asset>, AppError> {
    if bytes.len() > 64 * 1024 {
        return Err(response_error());
    }
    let version = parse_tag(tag).ok_or_else(response_error)?;
    let text = std::str::from_utf8(bytes).map_err(|_| response_error())?;
    let mut out = vec![];
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 2 || !valid_digest(fields[0]) {
            return Err(response_error());
        }
        let name = fields[1].trim_start_matches('*');
        if name.contains('/') || name.contains("..") || name.contains('\\') {
            return Err(response_error());
        }
        if ["aarch64", "x86_64", "universal"]
            .iter()
            .any(|arch| name == format!("AgentUpdate-{version}-macos-{arch}.dmg"))
        {
            if out.iter().any(|a: &Asset| a.name == name) {
                return Err(response_error());
            }
            out.push(Asset {
                name: name.into(),
                url: format!("{SOURCE}/download/{tag}/{name}"),
                sha256: Some(fields[0].to_lowercase()),
            });
        }
    }
    Ok(out)
}
fn manifest(tag: &str) -> Result<Vec<Asset>, AppError> {
    parse_tag(tag).ok_or_else(response_error)?;
    for name in ["SHA256SUMS", "SHA256SUMS.txt"] {
        if let Ok(bytes) =
            crate::artifacts::small_document(&format!("{SOURCE}/download/{tag}/{name}"), 64 * 1024)
        {
            return parse_checksums(&bytes, tag);
        }
    }
    Err(AppError::new(
        "RELEASE_CHECKSUM_MISSING",
        "发布未提供可信 SHA256SUMS 校验文件，请使用下载入口。",
    ))
}
pub fn fallback_from_location(location: &str, bytes: &[u8]) -> Result<ReleaseInfo, AppError> {
    let tag = location
        .strip_prefix(&format!("{SOURCE}/tag/"))
        .ok_or_else(response_error)?;
    let version = parse_tag(tag).ok_or_else(response_error)?;
    let assets = parse_checksums(bytes, tag)?;
    Ok(ReleaseInfo {
        version: version.to_string(),
        tag: tag.into(),
        published_at: String::new(),
        notes: "GitHub API 暂不可用；版本与安装包来自官方最新稳定 Release 和 SHA256SUMS。".into(),
        url: location.into(),
        assets,
    })
}
fn fallback() -> Result<FetchResult, FetchFailure> {
    let response = crate::artifacts::response(&format!("{SOURCE}/latest"), 20)
        .map_err(|_| failure("RELEASE_FALLBACK", 0))?;
    let location = response.url().as_str();
    let tag = location
        .strip_prefix(&format!("{SOURCE}/tag/"))
        .filter(|s| parse_tag(s).is_some())
        .ok_or_else(|| failure("RELEASE_FALLBACK", 0))?;
    let version = parse_tag(tag).unwrap();
    // Missing checksums preserve the release page, but never enable automatic installation.
    let assets = manifest(tag).unwrap_or_default();
    Ok(FetchResult::Published(
        Some(ReleaseInfo {
            version: version.to_string(),
            tag: tag.into(),
            published_at: String::new(),
            notes: "GitHub API 暂不可用，已通过官方 /releases/latest 确认版本。".into(),
            url: location.into(),
            assets,
        }),
        None,
    ))
}
pub fn install_package(
    root: &Path,
    current: &str,
    arch: &str,
) -> Result<crate::deploy::Package, AppError> {
    let result = check(root, true, current, arch)?;
    if result.state != "update_available" {
        return Err(AppError::new(
            "RELEASE_NOT_NEWER",
            "没有已确认的新稳定版本。",
        ));
    }
    let latest = result.latest.ok_or_else(response_error)?;
    let asset = result
        .asset
        .ok_or_else(|| AppError::new("RELEASE_ASSET_MISSING", "没有匹配的可信安装包。"))?;
    let sha256 = if let Some(digest) = asset.sha256.filter(|s| valid_digest(s)) {
        digest
    } else {
        manifest(&latest.tag)?
            .into_iter()
            .find(|a| a.name == asset.name)
            .and_then(|a| a.sha256)
            .ok_or_else(response_error)?
    };
    Ok(crate::deploy::Package {
        version: latest.version,
        url: asset.url,
        sha256,
        zip: false,
    })
}
