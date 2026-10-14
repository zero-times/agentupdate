use crate::{error::AppError, platform::process};
use std::{path::Path, time::Duration};

pub fn proxy() -> Option<String> {
    for key in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    let text = process::run(
        Path::new("/usr/sbin/scutil"),
        &["--proxy".into()],
        Path::new("/"),
        &[],
        Duration::from_secs(2),
        8192,
    )
    .ok()?;
    let field = |name: &str| {
        text.lines().find_map(|line| {
            let (key, value) = line.trim().split_once(" : ")?;
            (key == name).then(|| value.trim().to_owned())
        })
    };
    if field("HTTPSEnable").as_deref() != Some("1") {
        return None;
    }
    let host = field("HTTPSProxy")?;
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c))
    {
        return None;
    }
    let port: u16 = field("HTTPSPort")?.parse().ok()?;
    Some(format!("http://{host}:{port}"))
}
pub fn client(connect: Duration, timeout: Duration) -> Result<reqwest::blocking::Client, AppError> {
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(connect)
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none());
    if let Some(url) = proxy() {
        builder = builder.proxy(
            reqwest::Proxy::all(url)
                .map_err(|_| AppError::new("NETWORK_PROXY", "代理设置无效。"))?,
        );
    }
    builder
        .build()
        .map_err(|_| AppError::new("NETWORK_SETUP", "无法建立网络连接。"))
}
