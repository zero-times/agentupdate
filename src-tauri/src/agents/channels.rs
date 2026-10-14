use crate::{error::AppError, platform::network};
use serde_json::Value;
use std::{io::Read, time::Duration};

pub fn claude_document(path: &str) -> Result<Vec<u8>, AppError> {
    let allowed = path == "claude-code-releases/latest"
        || path == "releases/darwin/universal/RELEASES.json"
        || path
            .strip_prefix("claude-code-releases/")
            .and_then(|p| p.strip_suffix("/manifest.json"))
            .is_some_and(|v| semver::Version::parse(v).is_ok_and(|v| v.pre.is_empty()));
    if !allowed {
        return Err(AppError::new("CHANNEL_PATH", "没有此官方版本文档。"));
    }
    let client = network::client(Duration::from_secs(8), Duration::from_secs(20))?;
    let response = client
        .get(format!("https://downloads.claude.ai/{path}"))
        .send()
        .map_err(|_| AppError::new("CHANNEL_NETWORK", "Claude 官方版本查询失败。"))?;
    if !response.status().is_success() {
        return Err(AppError::new(
            "CHANNEL_HTTP",
            "Claude 官方版本服务暂不可用。",
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AppError::new("CHANNEL_RESPONSE", "Claude 官方响应中断。"))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(AppError::new("CHANNEL_RESPONSE", "Claude 官方响应过大。"));
    }
    Ok(bytes)
}

// Public Homebrew cask metadata supplies version/architecture and upstream hashes.
// Download URLs are independently allowlisted by each product adapter.
pub fn cask(token: &str) -> Result<Value, AppError> {
    if !["codex", "claude-code", "claude", "codex-app", "chatgpt"].contains(&token) {
        return Err(AppError::new("CHANNEL_ID", "没有此产品的版本渠道。"));
    }
    let client = network::client(Duration::from_secs(8), Duration::from_secs(20))?;
    let response = client
        .get(format!("https://formulae.brew.sh/api/cask/{token}.json"))
        .send()
        .map_err(|_| AppError::new("CHANNEL_NETWORK", "公开版本目录连接失败，请重试。"))?;
    if !response.status().is_success() {
        return Err(AppError::new("CHANNEL_HTTP", "公开版本目录暂不可用。"));
    }
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AppError::new("CHANNEL_RESPONSE", "版本目录响应中断。"))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(AppError::new("CHANNEL_RESPONSE", "版本目录响应过大。"));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::new("CHANNEL_RESPONSE", "版本目录格式无效。"))?;
    if value["token"].as_str() != Some(token) || value["disabled"] == true {
        return Err(AppError::new(
            "CHANNEL_RESPONSE",
            "版本目录产品身份无效或已停用。",
        ));
    }
    Ok(value)
}
