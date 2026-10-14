use agentupdate::agents::{
    discovery::{discover, DiscoveryContext},
    versions, AgentId,
};
use base64::Engine;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn fixture_version(agent: AgentId, version: &str, prerelease: bool) -> serde_json::Value {
    let filename = agent.package().split('/').next_back().unwrap();
    serde_json::json!({"version":version,"dist":{"tarball":format!("https://registry.npmjs.org/{}/-/{}-{}.tgz",agent.package(),filename,version),"integrity":format!("sha512-{}",base64::engine::general_purpose::STANDARD.encode([7u8;64]))},"prerelease":prerelease})
}

#[test]
fn official_versions_are_sorted_and_prereleases_require_opt_in() {
    let mut metadata =
        serde_json::json!({"name":"@openai/codex","dist-tags":{"latest":"0.10.0"},"versions":{}});
    for version in ["0.9.0", "0.10.0", "0.11.0-beta.1"] {
        metadata["versions"][version] =
            fixture_version(AgentId::Codex, version, version.contains('-'));
    }
    let stable = versions::parse(AgentId::Codex, &metadata, false).unwrap();
    assert_eq!(stable.versions[0].version, "0.10.0");
    assert_eq!(
        versions::select(&stable, "latest", false).unwrap().version,
        "0.10.0"
    );
    assert!(versions::select(&stable, "9.9.9", false).is_err());
    let all = versions::parse(AgentId::Codex, &metadata, true).unwrap();
    assert!(versions::select(&all, "0.11.0-beta.1", false).is_err());
    assert!(versions::select(&all, "0.11.0-beta.1", true).is_ok());
    metadata["versions"]["0.10.0"]["dist"]["tarball"] = "https://example.com/unsafe.tgz".into();
    assert!(versions::parse(AgentId::Codex, &metadata, false).is_err());
}

#[test]
fn wrong_package_or_integrity_never_produces_a_target() {
    let mut metadata = serde_json::json!({"name":"@google/gemini-cli","dist-tags":{"latest":"1.0.0"},"versions":{"1.0.0":fixture_version(AgentId::Gemini,"1.0.0",false)}});
    assert!(versions::parse(AgentId::Codex, &metadata, false).is_err());
    metadata["versions"]["1.0.0"]["dist"]["integrity"] = "sha512-broken".into();
    assert!(versions::parse(AgentId::Gemini, &metadata, false).is_err());
}

#[test]
fn homebrew_and_native_codex_versions_are_read_even_when_updates_are_disabled() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let home =
        std::env::temp_dir().join(format!("agentupdate-cask-version-{}", uuid::Uuid::new_v4()));
    let cask = home.join("brew/Caskroom/codex/0.162.1");
    let native = home.join(".local/bin");
    fs::create_dir_all(&cask).unwrap();
    fs::create_dir_all(&native).unwrap();
    fs::create_dir_all(home.join("brew/bin")).unwrap();
    for (path, version) in [
        (cask.join("codex"), "0.162.1"),
        (native.join("codex"), "0.162.0"),
    ] {
        fs::write(
            &path,
            format!("#!/bin/sh\nprintf 'codex-cli {version}\\n'\n"),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    symlink(cask.join("codex"), home.join("brew/bin/codex")).unwrap();
    let report = discover(
        &DiscoveryContext {
            home: home.clone(),
            directories: vec![home.join("brew/bin"), native],
        },
        &[],
    )
    .unwrap();
    assert_eq!(report.instances.len(), 2);
    assert!(report
        .instances
        .iter()
        .all(|i| i.version_verified && !i.supported));
    assert_eq!(report.instances[0].version.as_deref(), Some("0.162.1"));
    assert_eq!(report.instances[0].source, "homebrew");
    assert_eq!(report.instances[1].version.as_deref(), Some("0.162.0"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
#[ignore = "Read-only actual Mac discovery; no installation or credential changes"]
fn current_mac_versions() {
    let context = DiscoveryContext::current().unwrap();
    let report = discover(&context, &[]).unwrap();
    for agent in [AgentId::Codex, AgentId::Claude] {
        assert!(report
            .instances
            .iter()
            .any(|i| i.agent_id == agent && i.version_verified && i.version.is_some()));
    }
    if let Some(path) = std::env::var_os("AGENTUPDATE_DISCOVERY_EVIDENCE") {
        fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}

fn node_directory() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .find(|p| p.join("node").is_file())
        .unwrap()
}

fn fixture_install(home: &Path, name: &str, agent: AgentId, version: &str) -> PathBuf {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let prefix = home.join(name);
    let package = prefix.join("lib/node_modules").join(agent.package());
    fs::create_dir_all(&package).unwrap();
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::write(package.join("package.json"),serde_json::to_vec(&serde_json::json!({"name":agent.package(),"version":version,"bin":{agent.key():"cli.js"}})).unwrap()).unwrap();
    let launcher = package.join("cli.js");
    fs::write(
        &launcher,
        format!("#!/usr/bin/env node\nconsole.log('{}');\n", version),
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&launcher, prefix.join("bin").join(agent.key())).unwrap();
    symlink(node_directory().join("node"), prefix.join("bin/node")).unwrap();
    prefix
}

#[test]
fn isolated_discovery_preserves_multiple_instances_and_checks_executable_versions() {
    let home = std::env::temp_dir().join(format!(
        "agentupdate-discovery-test-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir(&home).unwrap();
    let first = fixture_install(&home, "prefix-a", AgentId::Codex, "0.10.0");
    let second = fixture_install(&home, "prefix-b", AgentId::Codex, "0.9.0");
    let gemini = fixture_install(&home, "prefix-c", AgentId::Gemini, "0.10.0");
    let context = DiscoveryContext {
        home: home.clone(),
        directories: vec![first.join("bin"), second.join("bin"), gemini.join("bin")],
    };
    let report = discover(&context, &[]).unwrap();
    assert_eq!(report.instances.len(), 3);
    assert!(report
        .instances
        .iter()
        .all(|i| i.supported && i.version_verified));
    assert_ne!(report.instances[0].id, report.instances[1].id);
    fs::write(
        first.join("lib/node_modules/@openai/codex/cli.js"),
        "#!/usr/bin/env node\nconsole.log('0.8.0');\n",
    )
    .unwrap();
    let mismatch = discover(&context, &[]).unwrap();
    assert!(
        !mismatch
            .instances
            .iter()
            .find(|i| i.prefix.as_ref() == Some(&fs::canonicalize(&first).unwrap()))
            .unwrap()
            .supported
    );
    assert!(discover(&context, &[PathBuf::from("relative/path")]).is_err());
    fs::remove_dir_all(home).unwrap();
}

#[test]
#[ignore = "Live official npm registry query; requires network"]
fn live_official_versions() {
    let mut results = Vec::new();
    for agent in [AgentId::Codex, AgentId::Gemini] {
        let catalog = versions::fetch(agent, false).unwrap();
        let selected = versions::select(&catalog, "latest", false).unwrap();
        assert!(!selected.prerelease);
        assert!(!catalog.versions.is_empty());
        results.push(serde_json::json!({"agent":agent,"latest":selected.version,"versionCount":catalog.versions.len(),"officialSource":true}));
    }
    if let Some(path) = std::env::var_os("AGENTUPDATE_VERSION_EVIDENCE") {
        fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "Requires isolated real npm fixtures created by the integration script"]
fn real_npm_instances() {
    let root = PathBuf::from(std::env::var_os("AGENTUPDATE_DISCOVERY_FIXTURE").unwrap());
    let context = DiscoveryContext {
        home: root.clone(),
        directories: vec![root.join("codex/bin"), root.join("gemini/bin")],
    };
    let report = discover(&context, &[]).unwrap();
    assert_eq!(report.instances.len(), 2);
    assert!(report
        .instances
        .iter()
        .all(|i| i.supported && i.version_verified));
    if let Some(path) = std::env::var_os("AGENTUPDATE_DISCOVERY_EVIDENCE") {
        let summary:Vec<_>=report.instances.iter().map(|i|serde_json::json!({"agent":i.agent_id,"source":i.source,"version":i.version,"architecture":i.architecture,"supported":i.supported,"versionVerified":i.version_verified})).collect();
        fs::write(path, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    }
}
