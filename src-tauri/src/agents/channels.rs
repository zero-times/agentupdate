use crate::{error::AppError, platform::network};
use serde_json::Value;
use std::{io::Read, time::Duration};

// Retry transient metadata failures once, including an interrupted response body.
// Redirects and invalid documents still fail closed.
fn document(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>, AppError> {
    for attempt in 0..2 {
        let result = (|| {
            let response = client.get(url).send().map_err(|_| {
                AppError::new(
                    "CHANNEL_NETWORK",
                    "版本查询连接失败，请检查网络或代理后重试。",
                )
            })?;
            let status = response.status();
            if !status.is_success() {
                return Err(AppError::new(
                    if status.is_server_error() {
                        "CHANNEL_TEMPORARY"
                    } else {
                        "CHANNEL_HTTP"
                    },
                    "公开版本服务暂不可用，请稍后重试。",
                ));
            }
            let mut bytes = Vec::new();
            response
                .take(2 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| AppError::new("CHANNEL_NETWORK", "版本目录响应中断，请重试。"))?;
            if bytes.len() > 2 * 1024 * 1024 {
                return Err(AppError::new("CHANNEL_RESPONSE", "版本目录响应过大。"));
            }
            Ok(bytes)
        })();
        match result {
            Err(error)
                if attempt == 0
                    && matches!(error.code, "CHANNEL_NETWORK" | "CHANNEL_TEMPORARY") =>
            {
                std::thread::sleep(Duration::from_millis(400));
            }
            Err(error) if error.code == "CHANNEL_TEMPORARY" => {
                return Err(AppError::new("CHANNEL_HTTP", error.message));
            }
            result => return result,
        }
    }
    unreachable!()
}

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
    document(&client, &format!("https://downloads.claude.ai/{path}"))
}

// Public Homebrew cask metadata supplies version/architecture and upstream hashes.
// Download URLs are independently allowlisted by each product adapter.
pub fn cask(token: &str) -> Result<Value, AppError> {
    if !["codex", "claude-code", "claude", "codex-app", "chatgpt"].contains(&token) {
        return Err(AppError::new("CHANNEL_ID", "没有此产品的版本渠道。"));
    }
    let client = network::client(Duration::from_secs(8), Duration::from_secs(20))?;
    let bytes = document(
        &client,
        &format!("https://formulae.brew.sh/api/cask/{token}.json"),
    )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    fn serve(responses: Vec<&'static str>) -> (String, std::thread::JoinHandle<()>) {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.local_addr().unwrap());
        let job = std::thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0; 4096];
                stream.read(&mut request).unwrap();
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (url, job)
    }
    fn client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }
    #[test]
    fn metadata_recovers_after_transient_status_or_interrupted_body() {
        for failure in [
            "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\nshort",
        ] {
            let (url, job) = serve(vec![
                failure,
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            ]);
            assert_eq!(document(&client(), &url).unwrap(), b"{}");
            job.join().unwrap();
        }
    }
    #[test]
    fn permanent_errors_and_redirects_are_not_retried() {
        for response in [
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 302 Found\r\nLocation: https://example.com\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ] {
            let (url, job) = serve(vec![response]);
            assert_eq!(document(&client(), &url).unwrap_err().code, "CHANNEL_HTTP");
            job.join().unwrap();
        }
    }
}
