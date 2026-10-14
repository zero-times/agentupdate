use crate::{agents::channels, error::AppError, platform::process};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Product {
    Codex,
    Claude,
    Chatgpt,
}
impl Product {
    fn token(self) -> &'static str {
        match self {
            Self::Codex => "codex-app",
            Self::Claude => "claude",
            Self::Chatgpt => "chatgpt",
        }
    }
    fn bundle(self) -> &'static str {
        match self {
            Self::Codex => "com.openai.codex",
            Self::Claude => "com.anthropic.claudefordesktop",
            Self::Chatgpt => "com.openai.chat",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex app",
            Self::Claude => "Claude app",
            Self::Chatgpt => "ChatGPT / 集成 Codex",
        }
    }
    fn page(self) -> &'static str {
        match self {
            Self::Codex => "https://openai.com/codex/",
            Self::Claude => "https://claude.ai/download",
            Self::Chatgpt => "https://chatgpt.com/download/",
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalApp {
    pub path: PathBuf,
    pub version: Option<String>,
    pub build: Option<String>,
    pub state: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCheck {
    pub product: Product,
    pub name: &'static str,
    pub installed: Vec<LocalApp>,
    pub latest_version: Option<String>,
    pub state: String,
    pub checked_at: u64,
    pub metadata_source: String,
    pub official_page: &'static str,
    pub download_available: bool,
    pub error_code: Option<String>,
    pub channel_note: &'static str,
}
fn version_parts(version: &str) -> Option<Vec<u64>> {
    let version = version.trim().trim_start_matches('v').split(',').next()?;
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() < 2
        || parts.len() > 8
        || parts
            .iter()
            .any(|p| p.is_empty() || p.len() > 16 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    parts.into_iter().map(|p| p.parse().ok()).collect()
}
pub fn compare(local: &str, latest: &str) -> Option<Ordering> {
    let (mut left, mut right) = (version_parts(local)?, version_parts(latest)?);
    let size = left.len().max(right.len());
    left.resize(size, 0);
    right.resize(size, 0);
    Some(left.cmp(&right))
}
pub fn parse_upstream(
    product: Product,
    value: &serde_json::Value,
    arch: &str,
) -> Result<(String, Option<String>), AppError> {
    let fail = || {
        AppError::new(
            "APP_CHANNEL_RESPONSE",
            "应用版本目录身份、版本或下载来源无法确认。",
        )
    };
    if value["token"].as_str() != Some(product.token()) || value["disabled"] == true {
        return Err(fail());
    }
    let version = value["version"]
        .as_str()
        .and_then(|v| v.split(',').next())
        .ok_or_else(fail)?;
    version_parts(version).ok_or_else(fail)?;
    let metadata = if arch == "x86_64" {
        value.pointer("/variations/sequoia").unwrap_or(value)
    } else {
        value
    };
    let raw = metadata["url"].as_str().ok_or_else(fail)?;
    let url = reqwest::Url::parse(raw).map_err(|_| fail())?;
    let allowed = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && match product {
            Product::Codex => {
                url.host_str() == Some("persistent.oaistatic.com")
                    && url.path().starts_with("/codex-app-prod/Codex-")
            }
            Product::Claude => {
                url.host_str() == Some("downloads.claude.ai")
                    && url.path().starts_with("/releases/darwin/")
            }
            Product::Chatgpt => {
                url.host_str() == Some("persistent.oaistatic.com")
                    && (url.path().starts_with("/sidekick/public/")
                        || url.path().starts_with("/codex-app-prod/ChatGPT-"))
            }
        };
    let digest = metadata["sha256"].as_str().ok_or_else(fail)?;
    if !allowed || digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(fail());
    }
    let compatible = if product != Product::Claude && arch == "x86_64" {
        url.path().contains("x64")
            || url.path().contains("x86_64")
            || url.path().contains("universal")
    } else {
        true
    };
    Ok((version.into(), compatible.then(|| raw.into())))
}
pub fn parse_claude_release(
    value: &serde_json::Value,
) -> Result<(String, Option<String>), AppError> {
    let fail = || AppError::new("APP_CHANNEL_RESPONSE", "Claude 官方更新文档无法确认。");
    let version = value["currentRelease"].as_str().ok_or_else(fail)?;
    version_parts(version).ok_or_else(fail)?;
    let entry = value["releases"]
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|e| e["version"].as_str() == Some(version))
        })
        .ok_or_else(fail)?;
    if entry["updateTo"]["version"].as_str() != Some(version) {
        return Err(fail());
    }
    let raw = entry["updateTo"]["url"].as_str().ok_or_else(fail)?;
    let url = reqwest::Url::parse(raw).map_err(|_| fail())?;
    if url.scheme() != "https"
        || url.host_str() != Some("downloads.claude.ai")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !url
            .path()
            .starts_with(&format!("/releases/darwin/universal/{version}/Claude-"))
    {
        return Err(fail());
    }
    Ok((version.into(), Some(raw.into())))
}
fn upstream(product: Product, arch: &str) -> Result<(String, Option<String>), AppError> {
    if product == Product::Claude {
        let bytes = channels::claude_document("releases/darwin/universal/RELEASES.json")?;
        let value = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::new("APP_CHANNEL_RESPONSE", "Claude 官方更新响应无效。"))?;
        parse_claude_release(&value)
    } else {
        parse_upstream(product, &channels::cask(product.token())?, arch)
    }
}
pub fn discover(home: &Path) -> Vec<(Product, LocalApp)> {
    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for directory in [PathBuf::from("/Applications"), home.join("Applications")] {
        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten().take(256) {
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with(".agentupdate-"))
                    || path.extension().is_none_or(|s| s != "app")
                {
                    continue;
                }
                let Ok(path) = fs::canonicalize(path) else {
                    continue;
                };
                if !seen.insert(path.clone()) {
                    continue;
                }
                let plist = path.join("Contents/Info.plist");
                if !fs::metadata(&plist).is_ok_and(|m| m.len() <= 512 * 1024) {
                    continue;
                }
                let Ok(text) = process::run(
                    Path::new("/usr/bin/plutil"),
                    &[
                        "-convert".into(),
                        "json".into(),
                        "-o".into(),
                        "-".into(),
                        plist.display().to_string(),
                    ],
                    Path::new("/"),
                    &[],
                    Duration::from_secs(2),
                    512 * 1024,
                ) else {
                    continue;
                };
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                if let Some(product) = identify(&value) {
                    found.push((
                        product,
                        LocalApp {
                            path,
                            version: value["CFBundleShortVersionString"]
                                .as_str()
                                .map(str::to_owned),
                            build: value["CFBundleVersion"].as_str().map(str::to_owned),
                            state: "not_checked".into(),
                        },
                    ));
                }
            }
        }
    }
    found.sort_by(|a, b| a.1.path.cmp(&b.1.path));
    found
}
fn identify(value: &serde_json::Value) -> Option<Product> {
    let identifier = value["CFBundleIdentifier"].as_str()?;
    if identifier == Product::Codex.bundle()
        && ["CFBundleName", "CFBundleDisplayName", "CFBundleExecutable"]
            .iter()
            .any(|key| value[key].as_str() == Some("ChatGPT"))
    {
        return Some(Product::Chatgpt);
    }
    [Product::Codex, Product::Claude, Product::Chatgpt]
        .into_iter()
        .find(|product| identifier == product.bundle())
}
pub fn check(home: &Path, arch: &str) -> Vec<AppCheck> {
    let installed = discover(home);
    std::thread::scope(|scope| {
        let jobs: Vec<_> = [Product::Codex, Product::Claude, Product::Chatgpt]
            .into_iter()
            .map(|product| {
                let mut local: Vec<_> = installed
                    .iter()
                    .filter(|(p, _)| *p == product)
                    .map(|(_, i)| i.clone())
                    .collect();
                scope.spawn(move || {
                    let checked_at = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let result = upstream(product, arch);
                    let (latest_version, download_available, error_code, state) = match result {
                        Ok((version, url)) => {
                            for i in &mut local {
                                i.state =
                                    match i.version.as_deref().and_then(|v| compare(v, &version)) {
                                        Some(Ordering::Less) => "update_available",
                                        Some(Ordering::Equal) => "up_to_date",
                                        Some(Ordering::Greater) => "local_newer",
                                        None => "unknown_version",
                                    }
                                    .into();
                            }
                            let state = if local.is_empty() {
                                "not_installed"
                            } else if local.iter().any(|i| i.state == "update_available") {
                                "update_available"
                            } else if local.iter().any(|i| i.state == "unknown_version") {
                                "unknown_version"
                            } else if local.iter().any(|i| i.state == "local_newer") {
                                "local_newer"
                            } else {
                                "up_to_date"
                            };
                            (Some(version), url.is_some(), None, state)
                        }
                        Err(error) => {
                            for i in &mut local {
                                i.state = "failed".into();
                            }
                            (None, false, Some(error.code.to_owned()), "failed")
                        }
                    };
                    AppCheck {
                        product,
                        name: product.name(),
                        installed: local,
                        latest_version,
                        state: state.into(),
                        checked_at,
                        metadata_source: if product == Product::Claude {
                            "https://downloads.claude.ai/releases/darwin/universal/RELEASES.json"
                                .into()
                        } else {
                            format!("https://formulae.brew.sh/api/cask/{}.json", product.token())
                        },
                        official_page: product.page(),
                        download_available,
                        error_code,
                        channel_note: if product == Product::Claude {
                            "Claude 官方公开稳定更新渠道；账户或分批发布资格由官方应用决定。"
                        } else {
                            "公开 Homebrew 目录中的稳定版本；厂商分批发布和账户渠道可能不同。"
                        },
                    }
                })
            })
            .collect();
        jobs.into_iter()
            .map(|job| job.join().expect("desktop channel worker failed"))
            .collect()
    })
}
pub fn open(product: Product, download: bool, arch: &str) -> Result<(), AppError> {
    let url = if download {
        upstream(product, arch)?.1.ok_or_else(|| {
            AppError::new(
                "APP_ARCHITECTURE",
                "没有此架构的可确认安装包，请查看官方入口。",
            )
        })?
    } else {
        product.page().into()
    };
    process::run(
        Path::new("/usr/bin/open"),
        &[url],
        Path::new("/"),
        &[],
        Duration::from_secs(5),
        8192,
    )
    .map(|_| ())
}

/// Installation uses the hashed Homebrew catalog entry, independently constrained to
/// the product's official server. Claude's rolling feed can precede this checksum.
pub fn package(product: Product, arch: &str) -> Result<crate::deploy::Package, AppError> {
    let value = channels::cask(product.token())?;
    let (version, url) = parse_upstream(product, &value, arch)?;
    let url =
        url.ok_or_else(|| AppError::new("APP_ARCHITECTURE", "没有此架构的可信应用安装包。"))?;
    if product == Product::Claude {
        let (latest, latest_url) = upstream(product, arch)?;
        if latest != version || latest_url.as_deref() != Some(&url) {
            return Err(AppError::new(
                "APP_CHECKSUM_PENDING",
                "Claude 官方发布领先于公开校验目录，请稍后重试安装。",
            ));
        }
    }
    let metadata = if arch == "x86_64" {
        value.pointer("/variations/sequoia").unwrap_or(&value)
    } else {
        &value
    };
    Ok(crate::deploy::Package {
        version,
        url: url.clone(),
        sha256: metadata["sha256"].as_str().unwrap().into(),
        zip: url.ends_with(".zip"),
    })
}
