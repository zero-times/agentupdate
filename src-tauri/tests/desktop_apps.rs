use agentupdate::desktop_apps::{self, Product};
use serde_json::json;
use std::cmp::Ordering;

#[test]
fn product_identity_numeric_versions_and_download_architecture_are_checked() {
    assert_eq!(
        desktop_apps::compare("0.9.0", "0.10.0"),
        Some(Ordering::Less)
    );
    assert_eq!(
        desktop_apps::compare("26.1007.30912", "26.623.141536"),
        Some(Ordering::Greater)
    );
    assert_eq!(desktop_apps::compare("2.1.0", "2.1"), Some(Ordering::Equal));
    assert!(desktop_apps::compare("latest", "2.1.0").is_none());
    let mut value = json!({"token":"claude","version":"2.31226.1,buildhash","url":"https://downloads.claude.ai/releases/darwin/universal/2.31226.1/Claude-build.zip","sha256":"a".repeat(64)});
    let (version, download) =
        desktop_apps::parse_upstream(Product::Claude, &value, "x86_64").unwrap();
    assert_eq!(version, "2.31226.1");
    assert!(download.is_some());
    assert!(desktop_apps::parse_upstream(Product::Codex, &value, "aarch64").is_err());
    value["url"] = "https://example.org/Claude.zip".into();
    assert!(desktop_apps::parse_upstream(Product::Claude, &value, "aarch64").is_err());
    value = json!({"token":"codex-app","version":"26.623.141536","url":"https://persistent.oaistatic.com/codex-app-prod/Codex-darwin-arm64-26.623.141536.zip","sha256":"b".repeat(64)});
    assert!(
        desktop_apps::parse_upstream(Product::Codex, &value, "aarch64")
            .unwrap()
            .1
            .is_some()
    );
    assert!(
        desktop_apps::parse_upstream(Product::Codex, &value, "x86_64")
            .unwrap()
            .1
            .is_none()
    );
    value["url"] =
        "https://persistent.oaistatic.com/codex-app-prod/ChatGPT-darwin-arm64.zip".into();
    assert!(desktop_apps::parse_upstream(Product::Codex, &value, "aarch64").is_err());
}

#[test]
fn renamed_bundles_and_multiple_copies_are_discovered_by_identity() {
    let home = std::env::temp_dir().join(format!(
        "agentupdate-app-discovery-{}",
        uuid::Uuid::new_v4()
    ));
    let app = home.join("Applications/Renamed Agent.app/Contents");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("Info.plist"), br#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.openai.codex</string><key>CFBundleShortVersionString</key><string>26.623.141536</string><key>CFBundleVersion</key><string>123</string></dict></plist>"#).unwrap();
    let integrated = home.join("Applications/Renamed Chat.app/Contents");
    std::fs::create_dir_all(&integrated).unwrap();
    std::fs::write(integrated.join("Info.plist"), br#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.openai.codex</string><key>CFBundleName</key><string>ChatGPT</string><key>CFBundleShortVersionString</key><string>26.1007.30912</string></dict></plist>"#).unwrap();
    let found = desktop_apps::discover(&home);
    assert!(found.iter().any(|(p, i)| *p == Product::Codex
        && i.path.ends_with("Renamed Agent.app")
        && i.version.as_deref() == Some("26.623.141536")));
    assert!(found
        .iter()
        .any(|(product, installation)| *product == Product::Chatgpt
            && installation.path.ends_with("Renamed Chat.app")));
    assert!(!found
        .iter()
        .any(|(product, installation)| *product == Product::Codex
            && installation.path.ends_with("Renamed Chat.app")));
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
#[ignore = "Live read-only installed bundle and public channel check"]
fn current_mac_desktop_apps() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let results = desktop_apps::check(&home, std::env::consts::ARCH);
    if let Some(path) = std::env::var_os("AGENTUPDATE_DESKTOP_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    println!("{}", serde_json::to_string_pretty(&results).unwrap());
    assert_eq!(results.len(), 3);
    assert!(results
        .iter()
        .all(|r| r.error_code.is_none() && r.latest_version.is_some()));
    assert!(results
        .iter()
        .any(|r| r.product == Product::Claude && !r.installed.is_empty()));
    assert!(results
        .iter()
        .any(|r| r.product == Product::Chatgpt && !r.installed.is_empty()));
}
