use crate::{agents::AgentId, error::AppError};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{io::Read, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableVersion {
    pub version: String,
    pub prerelease: bool,
    pub tarball: String,
    pub integrity: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionCatalog {
    pub agent_id: AgentId,
    pub latest_stable: String,
    pub versions: Vec<AvailableVersion>,
}

pub fn fetch(agent: AgentId, include_prerelease: bool) -> Result<VersionCatalog, AppError> {
    #[cfg(feature = "desktop-validation")]
    if let Some(root) = std::env::var_os("AGENTUPDATE_TEST_CATALOG_DIR") {
        let bytes =
            std::fs::read(std::path::PathBuf::from(root).join(format!("{}.json", agent.key())))
                .map_err(|_| AppError::new("VERSION_RESPONSE", "测试官方目录不可用。"))?;
        let json = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::new("VERSION_RESPONSE", "测试官方目录格式无效。"))?;
        return parse(agent, &json, include_prerelease);
    }
    let client = crate::platform::network::client(Duration::from_secs(8), Duration::from_secs(15))?;
    fetch_with_client(agent, include_prerelease, &client)
}

fn fetch_with_client(
    agent: AgentId,
    include_prerelease: bool,
    client: &reqwest::blocking::Client,
) -> Result<VersionCatalog, AppError> {
    let url = format!(
        "https://registry.npmjs.org/{}",
        agent.package().replace('/', "%2f")
    );
    for attempt in 0..2 {
        let request = client
            .get(&url)
            .header("Accept", "application/vnd.npm.install-v1+json")
            .send();
        match request {
            Ok(response) if response.status().is_success() => {
                const LIMIT: u64 = 32 * 1024 * 1024;
                if response.content_length().is_some_and(|s| s > LIMIT) {
                    return Err(AppError::new("VERSION_RESPONSE", "版本响应超过允许范围。"));
                }
                let mut bytes = Vec::new();
                response
                    .take(LIMIT + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| AppError::new("VERSION_NETWORK", "版本查询中断，请重试。"))?;
                if bytes.len() as u64 > LIMIT {
                    return Err(AppError::new("VERSION_RESPONSE", "版本响应超过允许范围。"));
                }
                let json = serde_json::from_slice(&bytes)
                    .map_err(|_| AppError::new("VERSION_RESPONSE", "官方版本响应格式无效。"))?;
                return parse(agent, &json, include_prerelease);
            }
            Ok(response)
                if (response.status().is_server_error() || response.status().as_u16() == 429)
                    && attempt == 0 => {}
            Ok(response) => {
                return Err(AppError::new(
                    if response.status().as_u16() == 429 {
                        "VERSION_RATE_LIMIT"
                    } else {
                        "VERSION_HTTP"
                    },
                    "官方版本服务暂不可用；未生成更新目标。",
                ))
            }
            Err(_) if attempt == 0 => {}
            Err(_) => {
                return Err(AppError::new(
                    "VERSION_NETWORK",
                    "版本查询失败，请检查网络；未生成更新目标。",
                ))
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_proxy_is_a_network_error_after_bounded_retries() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let client = reqwest::blocking::Client::builder()
            .proxy(reqwest::Proxy::all(format!("http://{address}")).unwrap())
            .connect_timeout(Duration::from_millis(100))
            .timeout(Duration::from_millis(100))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let start = std::time::Instant::now();
        let error = fetch_with_client(AgentId::Codex, false, &client).unwrap_err();
        assert_eq!(error.code, "VERSION_NETWORK");
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}

pub fn parse(
    agent: AgentId,
    json: &serde_json::Value,
    include_prerelease: bool,
) -> Result<VersionCatalog, AppError> {
    if json.get("name").and_then(|v| v.as_str()) != Some(agent.package()) {
        return Err(AppError::new(
            "VERSION_RESPONSE",
            "版本响应的工具身份不匹配。",
        ));
    }
    let entries = json
        .get("versions")
        .and_then(|v| v.as_object())
        .ok_or_else(|| AppError::new("VERSION_RESPONSE", "没有有效的版本列表。"))?;
    let mut versions = Vec::new();
    for (key, value) in entries {
        let Ok(parsed) = semver::Version::parse(key) else {
            continue;
        };
        if value.get("version").and_then(|v| v.as_str()) != Some(key.as_str()) {
            continue;
        }
        let prerelease = !parsed.pre.is_empty();
        if prerelease && !include_prerelease {
            continue;
        }
        let Some(tarball) = value.pointer("/dist/tarball").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(integrity) = value.pointer("/dist/integrity").and_then(|v| v.as_str()) else {
            continue;
        };
        let filename = agent.package().split('/').next_back().unwrap();
        let expected = format!(
            "https://registry.npmjs.org/{}/-/{}-{}.tgz",
            agent.package(),
            filename,
            parsed
        );
        let digest_valid = integrity
            .strip_prefix("sha512-")
            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
            .is_some_and(|bytes| bytes.len() == 64);
        if tarball != expected || !digest_valid {
            continue;
        }
        versions.push(AvailableVersion {
            version: parsed.to_string(),
            prerelease,
            tarball: tarball.into(),
            integrity: integrity.into(),
        });
    }
    versions.sort_by(|a, b| {
        semver::Version::parse(&b.version)
            .unwrap()
            .cmp(&semver::Version::parse(&a.version).unwrap())
    });
    let latest = json
        .pointer("/dist-tags/latest")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::new("VERSION_RESPONSE", "官方稳定渠道缺失。"))?;
    if !versions.iter().any(|v| {
        v.version == latest && (!v.prerelease || (agent == AgentId::Deepseek && include_prerelease))
    }) {
        return Err(AppError::new(
            "VERSION_RESPONSE",
            "官方稳定渠道没有有效制品。",
        ));
    }
    Ok(VersionCatalog {
        agent_id: agent,
        latest_stable: latest.into(),
        versions,
    })
}

pub fn select(
    catalog: &VersionCatalog,
    target: &str,
    allow_prerelease: bool,
) -> Result<AvailableVersion, AppError> {
    let target = if target == "latest" {
        &catalog.latest_stable
    } else {
        target
    };
    let value = catalog
        .versions
        .iter()
        .find(|v| v.version == target)
        .ok_or_else(|| {
            AppError::new(
                "VERSION_NOT_FOUND",
                "指定版本不存在或没有可验证的官方制品。",
            )
        })?;
    if value.prerelease && !allow_prerelease {
        return Err(AppError::new(
            "PRERELEASE_OPT_IN",
            "预发行版本必须显式选择。",
        ));
    }
    Ok(value.clone())
}
