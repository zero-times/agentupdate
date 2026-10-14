use agentupdate::{
    agents::{
        discovery::{self, DiscoveryContext},
        versions, AgentId,
    },
    artifacts,
    deploy::{self, BundleProduct},
    releases,
    updater::native,
};
use base64::Engine;
use std::{
    fs,
    path::{Path, PathBuf},
};
#[test]
fn deepseek_latest_rc_requires_explicit_pre_release_selection() {
    let v = "0.2.0-rc.2";
    let json = serde_json::json!({"name":"@deepseek-ai/dsh","dist-tags":{"latest":v},"versions":{v:{"version":v,"dist":{"tarball":format!("https://registry.npmjs.org/@deepseek-ai/dsh/-/dsh-{v}.tgz"),"integrity":format!("sha512-{}",base64::engine::general_purpose::STANDARD.encode([1;64]))}}}});
    assert!(versions::parse(AgentId::Deepseek, &json, false).is_err());
    let c = versions::parse(AgentId::Deepseek, &json, true).unwrap();
    assert!(versions::select(&c, "latest", false).is_err());
    assert!(versions::select(&c, "latest", true).unwrap().prerelease);
}
#[test]
fn codex_cask_accepts_only_official_architecture_and_simple_artifacts() {
    let mut value = serde_json::json!({"token":"codex","version":"0.162.1","url":"https://github.com/openai/codex/releases/download/rust-v0.162.1/codex-package-aarch64-apple-darwin.tar.gz","sha256":"ab".repeat(32),"artifacts":[{"binary":["bin/codex"],"target":"$HOMEBREW_PREFIX/bin/codex"},{"generate_completions_from_executable":["bin/codex","completion",{"shells":["bash","zsh","fish"]}]},{"zap":[{}]}]});
    assert!(native::codex_cask(&value, "arm64").is_ok());
    assert!(native::codex_cask(&value, "x64").is_err());
    value["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"postflight":["shell"]}));
    assert!(native::codex_cask(&value, "arm64").is_err());
    value["artifacts"].as_array_mut().unwrap().pop();
    value["url"] = "https://example.com/codex.tgz".into();
    assert!(native::codex_cask(&value, "arm64").is_err());
}
#[test]
fn release_fallback_keeps_verified_asset_and_rejects_manifest_traversal_or_duplicates() {
    let name = "AgentUpdate-0.2.0-macos-universal.dmg";
    let line = format!("{}  {name}\n", "a1".repeat(32));
    let r = releases::fallback_from_location(
        &format!("{}/tag/v0.2.0", releases::SOURCE),
        line.as_bytes(),
    )
    .unwrap();
    assert_eq!(
        r.assets[0].sha256.as_deref(),
        Some("a1".repeat(32).as_str())
    );
    assert!(releases::fallback_from_location(
        "https://github.com/other/project/releases/tag/v0.2.0",
        line.as_bytes()
    )
    .is_err());
    for text in [
        line.repeat(2),
        format!("{}  ../{name}\n", "a1".repeat(32)),
        format!("invalid  {name}\n"),
        format!("{}  folder/{name}\n", "a1".repeat(32)),
    ] {
        assert!(releases::parse_checksums(text.as_bytes(), "v0.2.0").is_err());
    }
    assert!(releases::parse_checksums(&vec![b'a'; 65537], "v0.2.0").is_err());
}
#[test]
fn redirect_and_bundle_identity_do_not_allow_other_products() {
    for u in [
        "http://github.com/a",
        "https://evil.test/a",
        "https://github.com@evil.test/a",
        "https://github.com:444/a",
    ] {
        assert!(!artifacts::redirect_allowed(
            &reqwest::Url::parse(u).unwrap()
        ));
    }
    let v = serde_json::json!({"CFBundleIdentifier":"com.openai.codex","CFBundleName":"ChatGPT","CFBundleExecutable":"ChatGPT"});
    assert!(!deploy::identity(BundleProduct::Codex, &v));
    assert!(deploy::identity(BundleProduct::Chatgpt, &v));
}
#[test]
fn bundle_install_destination_rejects_renamed_development_and_symlink_locations() {
    let home = std::env::temp_dir().join(format!("agentupdate-path-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join("Applications")).unwrap();
    let home = fs::canonicalize(home).unwrap();
    assert!(deploy::validate_destination(
        &home,
        BundleProduct::Codex,
        &home.join("Applications/Codex.app")
    )
    .is_ok());
    assert!(
        deploy::validate_destination(&home, BundleProduct::Codex, &home.join("dev/Codex.app"))
            .is_err()
    );
    assert!(deploy::validate_destination(
        &home,
        BundleProduct::Claude,
        &home.join("Applications/Codex.app")
    )
    .is_err());
    std::os::unix::fs::symlink("/Applications", home.join("linked")).unwrap();
    assert!(deploy::validate_destination(
        &home,
        BundleProduct::Claude,
        &home.join("linked/Claude.app")
    )
    .is_err());
    fs::remove_dir_all(home).unwrap();
}
#[test]
#[ignore = "Read-only real local CLI discovery and official channels; no daily installation writes"]
fn local_priority_tools_and_channels() {
    let context = DiscoveryContext::current().unwrap();
    let report = discovery::discover(&context, &[]).unwrap();
    let mut output = vec![];
    for agent in [AgentId::Codex, AgentId::Claude, AgentId::Deepseek] {
        let i = report
            .instances
            .iter()
            .find(|i| i.agent_id == agent && i.version_verified)
            .unwrap();
        assert!(i.supported, "{:?}: {:?}", agent, i.reason);
        let c = if native::handles(i) {
            native::catalog(i).unwrap()
        } else {
            versions::fetch(agent, agent == AgentId::Deepseek).unwrap()
        };
        let config_root = agentupdate::backup::config_root(&context.home, agent).unwrap();
        let config = agentupdate::backup::config_snapshot(&config_root, agent).unwrap();
        assert!(config.bytes <= 64 * 1024 * 1024);
        let old =
            agentupdate::backup::snapshot::scan(i.package_root.as_ref().unwrap(), None).unwrap();
        output.push(serde_json::json!({"agent":agent,"installed":i.version,"source":i.source,"latest":c.latest_stable,"supported":i.supported,"architecture":i.architecture,"configurationBackupReadable":true,"configurationBytes":config.bytes,"oldInstallationBytes":old.bytes}));
    }
    for p in [
        agentupdate::desktop_apps::Product::Codex,
        agentupdate::desktop_apps::Product::Claude,
    ] {
        let package = agentupdate::desktop_apps::package(p, std::env::consts::ARCH).unwrap();
        output.push(serde_json::json!({"desktop":p,"latest":package.version,"sha256":package.sha256,"trustedPackage":true}));
    }
    if let Some(path) = std::env::var_os("AGENTUPDATE_INSTALL_EVIDENCE") {
        fs::write(path, serde_json::to_vec_pretty(&output).unwrap()).unwrap();
    }
}
#[test]
#[ignore = "Real first installation into temporary HOME, official npm downloads; removes fixture after verifying rollback"]
fn real_missing_cli_install_and_rollback() {
    let root = std::env::temp_dir().join(format!("agentupdate-new-cli-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let ctx = DiscoveryContext {
        home: fs::canonicalize(&root).unwrap(),
        directories: vec![PathBuf::from("/opt/homebrew/bin")],
    };
    let data = root.join("data");
    let _store = agentupdate::storage::Store::open(&data).unwrap();
    let agent = match std::env::var("AGENTUPDATE_TEST_INSTALL_AGENT").as_deref() {
        Ok("codex") => AgentId::Codex,
        Ok("claude") => AgentId::Claude,
        Ok("gemini") => AgentId::Gemini,
        _ => AgentId::Deepseek,
    };
    let catalog = versions::fetch(agent, agent == AgentId::Deepseek).unwrap();
    let target = versions::select(&catalog, "latest", agent == AgentId::Deepseek).unwrap();
    let record = agentupdate::cli_install::install_target(
        &data,
        &ctx,
        agent,
        target,
        fs::canonicalize("/opt/homebrew/bin/node").unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(record.state, "succeeded");
    assert!(record.command.exists());
    let installed = discovery::discover(
        &DiscoveryContext {
            home: ctx.home.clone(),
            directories: vec![
                ctx.home.join(".local/bin"),
                PathBuf::from("/opt/homebrew/bin"),
            ],
        },
        &[],
    )
    .unwrap()
    .instances
    .into_iter()
    .find(|i| i.agent_id == agent && i.command_path == record.command)
    .unwrap();
    let npmc = agentupdate::updater::npm::Context {
        home: ctx.home.clone(),
        ..agentupdate::updater::npm::Context::current(&installed).unwrap()
    };
    agentupdate::updater::npm::preflight(&installed, &npmc).unwrap();
    let restored = agentupdate::cli_install::rollback(&data, &ctx.home, &record.id).unwrap();
    assert_eq!(restored.state, "restored");
    assert!(fs::symlink_metadata(record.command).is_err());
    assert!(!record.package.exists());
    if let Some(path) = std::env::var_os("AGENTUPDATE_INSTALL_EVIDENCE") {
        fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"agent":agent,"version":record.version,"installed":true,"rollbackToAbsent":true,"homeIsolated":true})).unwrap()).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}
fn fake_bundle(path: &Path, version: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
    let c = path
        .parent()
        .unwrap()
        .join(format!("fixture-{}.c", uuid::Uuid::new_v4()));
    fs::write(&c, "int main(){return 0;}\n").unwrap();
    assert!(std::process::Command::new("/usr/bin/cc")
        .arg(&c)
        .args(["-arch", "arm64", "-arch", "x86_64"])
        .arg("-o")
        .arg(path.join("Contents/MacOS/fixture"))
        .status()
        .unwrap()
        .success());
    fs::remove_file(c).unwrap();
    fs::write(path.join("Contents/Info.plist"),format!(r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>work.x5brain.agentupdate</string><key>CFBundleExecutable</key><string>fixture</string><key>CFBundleShortVersionString</key><string>{version}</string><key>CFBundleVersion</key><string>{version}</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>"#)).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(std::process::Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(path)
        .status()
        .unwrap()
        .success());
}
fn bundle_fixture() -> (PathBuf, PathBuf, deploy::Record) {
    let home =
        std::env::temp_dir().join(format!("agentupdate-bundle-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(home.join("Applications")).unwrap();
    let home = fs::canonicalize(home).unwrap();
    let data = home.join("data");
    let _store = agentupdate::storage::Store::open(&data).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let work = data.join("app-installs").join(&id);
    fs::create_dir_all(work.join("extracted")).unwrap();
    let destination = home.join("Applications/AgentUpdate.app");
    let staged = work.join("extracted/AgentUpdate.app");
    fake_bundle(&destination, "0.1.0");
    fake_bundle(&staged, "0.2.0");
    assert!(std::process::Command::new("/usr/bin/ditto")
        .arg(&destination)
        .arg(work.join("original.app"))
        .status()
        .unwrap()
        .success());
    let original = agentupdate::backup::snapshot::scan(&destination, None).unwrap();
    let staged = agentupdate::backup::snapshot::scan(&staged, None).unwrap();
    let config = agentupdate::backup::snapshot::scan(&home, Some(&[])).unwrap();
    let r = deploy::Record {
        id,
        product: BundleProduct::Agentupdate,
        version: "0.2.0".into(),
        destination,
        state: "prepared".into(),
        failure_code: None,
        created_at: 1,
        original: Some(original),
        staged,
        config,
        home: home.clone(),
    };
    agentupdate::platform::files::write_atomic(
        &work.join("record.json"),
        &serde_json::to_vec(&r).unwrap(),
    )
    .unwrap();
    (home, data, r)
}
#[test]
fn signed_bundle_swap_and_rollback_preserve_the_original() {
    let (home, data, r) = bundle_fixture();
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = events.clone();
    let result = agentupdate::progress::with_reporter(
        Some(std::sync::Arc::new(move |p| log.lock().unwrap().push(p))),
        || {
            agentupdate::progress::operation("app:agentupdate", || {
                deploy::install(&data, &home, &r.id, std::env::consts::ARCH)
            })
        },
    )
    .unwrap();
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.phase)
            .collect::<Vec<_>>(),
        vec![
            agentupdate::progress::Phase::Checking,
            agentupdate::progress::Phase::Installing,
            agentupdate::progress::Phase::Verifying,
            agentupdate::progress::Phase::Succeeded
        ]
    );
    assert_eq!(result.state, "succeeded");
    assert_eq!(
        deploy::plist(&r.destination).unwrap()["CFBundleShortVersionString"],
        "0.2.0"
    );
    let restored = deploy::rollback(&data, &home, &r.id).unwrap();
    assert_eq!(restored.state, "restored");
    agentupdate::backup::snapshot::verify_tree(&r.destination, r.original.as_ref().unwrap())
        .unwrap();
    fs::remove_dir_all(home).unwrap();
}
#[test]
fn altered_bundle_is_rejected_before_swap_and_interrupted_swap_can_restore() {
    let (home, data, mut r) = bundle_fixture();
    let work = data.join("app-installs").join(&r.id);
    let staged = work.join("extracted/AgentUpdate.app");
    fs::write(staged.join("Contents/MacOS/fixture"), "corrupt").unwrap();
    assert!(deploy::install(&data, &home, &r.id, std::env::consts::ARCH).is_err());
    agentupdate::backup::snapshot::verify_tree(&r.destination, r.original.as_ref().unwrap())
        .unwrap();
    let displaced = r
        .destination
        .parent()
        .unwrap()
        .join(format!(".agentupdate-old-{}.app", r.id));
    fs::rename(&r.destination, &displaced).unwrap();
    r.state = "installing".into();
    agentupdate::platform::files::write_atomic(
        &work.join("record.json"),
        &serde_json::to_vec(&r).unwrap(),
    )
    .unwrap();
    assert_eq!(
        deploy::rollback(&data, &home, &r.id).unwrap().state,
        "restored"
    );
    agentupdate::backup::snapshot::verify_tree(&r.destination, r.original.as_ref().unwrap())
        .unwrap();
    fs::remove_dir_all(home).unwrap();
}
#[test]
#[ignore = "Official Codex/Claude packages installed only under a temporary user's Applications, then restored to absent"]
fn official_desktop_install_roundtrip() {
    let product = if std::env::var("AGENTUPDATE_TEST_APP").as_deref() == Ok("codex") {
        BundleProduct::Codex
    } else {
        BundleProduct::Claude
    };
    let base = std::env::temp_dir().join(format!(
        "agentupdate-app-roundtrip-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(base.join("Applications")).unwrap();
    let home = fs::canonicalize(base).unwrap();
    let data = home.join("data");
    let _store = agentupdate::storage::Store::open(&data).unwrap();
    let package =
        agentupdate::desktop_apps::package(product.desktop().unwrap(), std::env::consts::ARCH)
            .unwrap();
    let r = deploy::prepare_at(
        &data,
        &home,
        product,
        &package,
        std::env::consts::ARCH,
        &home.join("Applications").join(product.name()),
    )
    .unwrap();
    assert!(r.original.is_none());
    assert_eq!(
        deploy::install(&data, &home, &r.id, std::env::consts::ARCH)
            .unwrap()
            .state,
        "succeeded"
    );
    deploy::verify_bundle(
        &r.destination,
        product,
        &package.version,
        std::env::consts::ARCH,
    )
    .unwrap();
    assert_eq!(
        deploy::rollback(&data, &home, &r.id).unwrap().state,
        "restored"
    );
    assert!(!r.destination.exists());
    if let Some(path) = std::env::var_os("AGENTUPDATE_INSTALL_EVIDENCE") {
        fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"product":product,"version":package.version,"sha256":package.sha256,"officialDownloadVerified":true,"vendorSignatureVerified":true,"installationVerified":true,"rollbackToAbsent":true,"homeIsolated":true})).unwrap()).unwrap();
    }
    fs::remove_dir_all(home).unwrap();
}
#[test]
#[ignore = "Uses the built production executable as a real detached helper, waiting for a fixture parent process to exit"]
fn production_helper_waits_installs_and_can_restore() {
    let binary = std::env::var_os("AGENTUPDATE_HELPER_BINARY").unwrap();
    let command = || {
        if std::env::var("AGENTUPDATE_HELPER_ARCH").as_deref() == Ok("x86_64") {
            let mut c = std::process::Command::new("/usr/bin/arch");
            c.arg("-x86_64").arg(&binary);
            c
        } else {
            std::process::Command::new(&binary)
        }
    };
    let (home, data, r) = bundle_fixture();
    let mut parent = std::process::Command::new("/bin/sleep")
        .arg("2")
        .spawn()
        .unwrap();
    let parent_pid = parent.id();
    let reap = std::thread::spawn(move || parent.wait().unwrap());
    let start = std::time::Instant::now();
    assert!(command()
        .args(["--install-helper"])
        .arg(&data)
        .arg(&r.id)
        .arg(parent_pid.to_string())
        .spawn()
        .unwrap()
        .wait()
        .unwrap()
        .success());
    assert!(start.elapsed() >= std::time::Duration::from_secs(2));
    reap.join().unwrap();
    let records = deploy::history(&data).unwrap();
    let installed = records.into_iter().find(|v| v.id == r.id).unwrap();
    assert_eq!(installed.state, "succeeded", "{:?}", installed.failure_code);
    assert_eq!(
        deploy::plist(&r.destination).unwrap()["CFBundleShortVersionString"],
        "0.2.0"
    );
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(command()
        .arg("--restore-helper")
        .arg(&data)
        .arg(&r.id)
        .arg("99999999")
        .status()
        .unwrap()
        .success());
    assert_eq!(deploy::history(&data).unwrap()[0].state, "restored");
    agentupdate::backup::snapshot::verify_tree(&r.destination, r.original.as_ref().unwrap())
        .unwrap();
    if let Some(path) = std::env::var_os("AGENTUPDATE_INSTALL_EVIDENCE") {
        fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"helperWaitedForParent":true,"headlessBundleReplacement":true,"restartRequested":true,"restoreHelperVerified":true,"dailyApplicationChanged":false})).unwrap()).unwrap();
    }
    fs::remove_dir_all(home).unwrap();
}
#[test]
fn harness_backup_covers_profile_configuration_without_following_runtime_projections() {
    let base =
        std::env::temp_dir().join(format!("agentupdate-dsh-config-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(base.join("profiles/web/node_modules")).unwrap();
    let base = fs::canonicalize(base).unwrap();
    fs::write(
        base.join(".credentials.yaml"),
        "synthetic fixture credential",
    )
    .unwrap();
    fs::write(base.join("profiles/web/cordis.patch.yml"), "fixture: true").unwrap();
    fs::write(base.join("profiles/web/package.json"), "{}").unwrap();
    std::os::unix::fs::symlink(
        "/opt/homebrew/lib/node_modules",
        base.join("profiles/web/node_modules/runtime"),
    )
    .unwrap();
    let snapshot = agentupdate::backup::config_snapshot(&base, AgentId::Deepseek).unwrap();
    assert!(snapshot
        .entries
        .iter()
        .any(|e| e.path == "profiles/web/cordis.patch.yml"));
    assert!(snapshot
        .entries
        .iter()
        .any(|e| e.path == ".credentials.yaml"));
    assert!(!snapshot
        .entries
        .iter()
        .any(|e| e.path.contains("node_modules")));
    fs::remove_dir_all(base).unwrap();
}
#[test]
fn harness_upgrade_and_rollback_keep_profile_config_and_runtime_links() {
    use agentupdate::{
        backup,
        storage::{
            model::{PlannedItem, Stage},
            Store,
        },
        updater::{self, npm, rollback},
    };
    use sha2::{Digest, Sha512};
    use std::os::unix::fs::{symlink, PermissionsExt};
    let base =
        std::env::temp_dir().join(format!("agentupdate-dsh-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(base.join("prefix/bin")).unwrap();
    let root = fs::canonicalize(base).unwrap();
    let package = root.join("prefix/lib/node_modules/@deepseek-ai/dsh");
    fs::create_dir_all(package.join("lib")).unwrap();
    let manifest = |v: &str| {
        serde_json::to_vec(
            &serde_json::json!({"name":"@deepseek-ai/dsh","version":v,"bin":{"dsh":"lib/bin.js"}}),
        )
        .unwrap()
    };
    let script = |v: &str| {
        format!("#!/usr/bin/env node\nconsole.log(process.argv.includes('--help')?'Harness help fixture':'{v}');\n")
    };
    fs::write(package.join("package.json"), manifest("1.0.0")).unwrap();
    fs::write(package.join("lib/bin.js"), script("1.0.0")).unwrap();
    fs::set_permissions(
        package.join("lib/bin.js"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    symlink(package.join("lib/bin.js"), root.join("prefix/bin/dsh")).unwrap();
    symlink("/opt/homebrew/bin/node", root.join("prefix/bin/node")).unwrap();
    fs::create_dir_all(root.join(".dsh/profiles/web/node_modules")).unwrap();
    fs::write(root.join(".dsh/.credentials.yaml"), "original fixture").unwrap();
    fs::write(
        root.join(".dsh/profiles/web/cordis.patch.yml"),
        "old profile fixture",
    )
    .unwrap();
    fs::write(root.join(".dsh/profiles/web/package.json"), "{}").unwrap();
    symlink(
        &package,
        root.join(".dsh/profiles/web/node_modules/runtime"),
    )
    .unwrap();
    let ctx = DiscoveryContext {
        home: root.clone(),
        directories: vec![root.join("prefix/bin")],
    };
    let i = discovery::discover(&ctx, &[])
        .unwrap()
        .instances
        .into_iter()
        .find(|i| i.agent_id == AgentId::Deepseek)
        .unwrap();
    assert!(i.supported);
    let target_root = root.join("target/package");
    fs::create_dir_all(target_root.join("lib")).unwrap();
    fs::write(target_root.join("package.json"), manifest("1.1.0")).unwrap();
    fs::write(target_root.join("lib/bin.js"), script("1.1.0")).unwrap();
    fs::set_permissions(
        target_root.join("lib/bin.js"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let tar = root.join("target.tgz");
    assert!(std::process::Command::new("/usr/bin/tar")
        .args(["-czf"])
        .arg(&tar)
        .arg("-C")
        .arg(target_root.parent().unwrap())
        .arg("package")
        .status()
        .unwrap()
        .success());
    let bytes = fs::read(tar).unwrap();
    let mut store = Store::open(&root.join("data")).unwrap();
    let plan = store
        .create_plan(vec![PlannedItem {
            installation: i.clone(),
            target: versions::AvailableVersion {
                version: "1.1.0".into(),
                prerelease: false,
                tarball: "https://registry.npmjs.org/@deepseek-ai/dsh/-/dsh-1.1.0.tgz".into(),
                integrity: format!(
                    "sha512-{}",
                    base64::engine::general_purpose::STANDARD.encode(Sha512::digest(&bytes))
                ),
            },
        }])
        .unwrap();
    let id = &plan.transactions[0].id;
    let work = store.root.join("work").join(id);
    agentupdate::platform::files::private_directory(&work).unwrap();
    agentupdate::platform::files::write_atomic(&work.join("target.tgz"), &bytes).unwrap();
    let npmc = npm::Context {
        home: root.clone(),
        npm: fs::canonicalize("/opt/homebrew/bin/npm").unwrap(),
        cache: None,
        offline: true,
    };
    assert_eq!(
        updater::execute_item(&mut store, id, &npmc).unwrap().stage,
        Stage::Succeeded
    );
    fs::write(
        root.join(".dsh/profiles/web/cordis.patch.yml"),
        "changed fixture",
    )
    .unwrap();
    fs::write(
        root.join(".dsh/.credentials.yaml"),
        "changed credential fixture",
    )
    .unwrap();
    assert_eq!(
        rollback::execute(&mut store, id, &npmc, true)
            .unwrap()
            .stage,
        Stage::Restored
    );
    assert_eq!(
        fs::read(root.join(".dsh/profiles/web/cordis.patch.yml")).unwrap(),
        b"old profile fixture"
    );
    assert_eq!(
        fs::read(root.join(".dsh/.credentials.yaml")).unwrap(),
        b"original fixture"
    );
    assert!(root
        .join(".dsh/profiles/web/node_modules/runtime")
        .is_symlink());
    assert_eq!(
        discovery::discover(&ctx, &[]).unwrap().instances[0]
            .version
            .as_deref(),
        Some("1.0.0")
    );
    assert!(backup::config_snapshot(&root.join(".dsh"), AgentId::Deepseek).is_ok());
    fs::remove_dir_all(root).unwrap();
}
#[test]
#[ignore = "Downloads and stages the official Codex cask archive; leaves the daily Homebrew installation unchanged"]
fn official_codex_cask_archive_stages_with_health_check() {
    let ctx = DiscoveryContext::current().unwrap();
    let i = discovery::discover(&ctx, &[])
        .unwrap()
        .instances
        .into_iter()
        .find(|i| i.agent_id == AgentId::Codex && i.source == "homebrew")
        .unwrap();
    let c = native::catalog(&i).unwrap();
    let target = versions::select(&c, "latest", false).unwrap();
    let root =
        std::env::temp_dir().join(format!("agentupdate-codex-stage-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let tx = agentupdate::storage::model::UpdateTransaction {
        id: uuid::Uuid::new_v4().to_string(),
        plan_id: uuid::Uuid::new_v4().to_string(),
        ordinal: 0,
        installation: i,
        target: target.clone(),
        stage: agentupdate::storage::model::Stage::Prepared,
        restore_point_id: None,
        failure_code: None,
        created_at: 1,
        updated_at: 1,
    };
    let context = agentupdate::updater::npm::Context::current(&tx.installation).unwrap();
    let staged = native::stage(&tx, &context, &root.join("stage")).unwrap();
    assert_eq!(staged.version.as_deref(), Some(target.version.as_str()));
    if let Some(path) = std::env::var_os("AGENTUPDATE_INSTALL_EVIDENCE") {
        fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"officialCaskVersion":target.version,"verifiedArchiveStaged":true,"actualVersionAndHelpPassed":true,"homebrewNotModified":true})).unwrap()).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}
