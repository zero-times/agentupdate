use agentupdate::{
    backup::snapshot::{self, Snapshot},
    platform::files,
};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::PathBuf,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("agentupdate-backup-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn snapshot_preserves_internal_links_modes_and_original_absence() {
    let f = Fixture::new();
    let source = f.0.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("settings.json"), b"private fixture").unwrap();
    fs::set_permissions(
        source.join("settings.json"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    symlink("settings.json", source.join("alias")).unwrap();
    let snap = snapshot::scan(&source, Some(&["settings.json", "alias", "absent"])).unwrap();
    assert!(snap
        .entries
        .iter()
        .any(|e| e.path == "absent" && e.kind == "missing"));
    let archive = f.0.join("archive");
    snapshot::copy(&source, &archive, &snap).unwrap();
    let restored = f.0.join("restored");
    snapshot::materialize(&archive, &snap, &restored).unwrap();
    assert_eq!(
        fs::read(restored.join("alias")).unwrap(),
        b"private fixture"
    );
    assert_eq!(
        fs::read_link(restored.join("alias")).unwrap(),
        PathBuf::from("settings.json")
    );
    assert!(!restored.join("absent").exists());
    assert_eq!(
        fs::metadata(restored.join("settings.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
}
#[test]
fn external_broken_links_and_unreadable_sources_are_rejected() {
    let f = Fixture::new();
    let root = f.0.join("root");
    fs::create_dir(&root).unwrap();
    fs::write(f.0.join("outside"), b"private").unwrap();
    symlink("../outside", root.join("bad")).unwrap();
    assert!(snapshot::scan(&root, None).is_err());
    fs::remove_file(root.join("bad")).unwrap();
    symlink("missing", root.join("bad")).unwrap();
    assert!(snapshot::scan(&root, None).is_err());
    fs::remove_file(root.join("bad")).unwrap();
    fs::write(root.join("file"), b"private").unwrap();
    fs::set_permissions(root.join("file"), fs::Permissions::from_mode(0o000)).unwrap();
    assert!(snapshot::scan(&root, None).is_err());
    fs::set_permissions(root.join("file"), fs::Permissions::from_mode(0o600)).unwrap();
}
#[test]
fn changed_sources_corrupt_payload_and_disk_full_do_not_publish() {
    let f = Fixture::new();
    let root = f.0.join("root");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file"), b"before").unwrap();
    let snap = snapshot::scan(&root, None).unwrap();
    fs::write(root.join("file"), b"after!").unwrap();
    assert!(snapshot::copy(&root, &f.0.join("changed"), &snap).is_err());
    fs::write(root.join("file"), b"before").unwrap();
    let archive = f.0.join("archive");
    snapshot::copy(&root, &archive, &snap).unwrap();
    let payload = fs::read_dir(&archive)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(payload, b"broken").unwrap();
    assert!(snapshot::verify(&archive, &snap).is_err());
    assert!(snapshot::require_space(1, 2).is_err());
    assert!(snapshot::require_space(2, 2).is_ok());
}
#[test]
fn missing_config_root_is_valid_and_paths_cannot_escape() {
    let f = Fixture::new();
    let root = f.0.join("missing");
    let snap = snapshot::scan(&root, Some(&["config.toml", "auth.json"])).unwrap();
    assert!(snap.entries.iter().all(|e| e.kind == "missing"));
    snapshot::copy(&root, &f.0.join("archive"), &snap).unwrap();
    let corrupt = Snapshot {
        entries: vec![snapshot::Entry {
            path: "../escape".into(),
            kind: "missing".into(),
            mode: 0,
            size: 0,
            sha256: None,
            link: None,
        }],
        bytes: 0,
    };
    assert!(snapshot::verify(&f.0.join("archive"), &corrupt).is_err());
    assert!(files::write_atomic(&f.0.join("simple"), b"private").is_ok());
}
#[test]
fn published_restore_contains_full_old_package_and_private_config() {
    use agentupdate::{
        agents::{
            discovery::{discover, DiscoveryContext},
            versions::AvailableVersion,
            AgentId,
        },
        backup,
        storage::{
            model::{PlannedItem, Stage},
            Store,
        },
    };
    let f = Fixture::new();
    let prefix = f.0.join("prefix");
    let package = prefix.join("lib/node_modules/@openai/codex");
    fs::create_dir_all(&package).unwrap();
    fs::create_dir(prefix.join("bin")).unwrap();
    fs::write(
        package.join("package.json"),
        br#"{"name":"@openai/codex","version":"1.0.0","bin":{"codex":"cli.js"}}"#,
    )
    .unwrap();
    fs::write(
        package.join("cli.js"),
        "#!/usr/bin/env node\nconsole.log('1.0.0');\n",
    )
    .unwrap();
    fs::set_permissions(package.join("cli.js"), fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&package.join("cli.js"), prefix.join("bin/codex")).unwrap();
    symlink("/opt/homebrew/bin/node", prefix.join("bin/node")).unwrap();
    fs::create_dir(f.0.join(".codex")).unwrap();
    fs::write(f.0.join(".codex/config.toml"), b"private fixture").unwrap();
    let report = discover(
        &DiscoveryContext {
            home: f.0.clone(),
            directories: vec![prefix.join("bin")],
        },
        &[],
    )
    .unwrap();
    let installation = report
        .instances
        .into_iter()
        .find(|i| i.agent_id == AgentId::Codex)
        .unwrap();
    assert!(installation.supported);
    let mut store = Store::open(&f.0.join("app-data")).unwrap();
    let plan = store
        .create_plan(vec![PlannedItem {
            installation,
            target: AvailableVersion {
                version: "1.1.0".into(),
                prerelease: false,
                tarball: String::new(),
                integrity: String::new(),
            },
        }])
        .unwrap();
    let id = &plan.transactions[0].id;
    store
        .transition(id, Stage::Queued, Stage::Preflight, None)
        .unwrap();
    let tx = store
        .transition(id, Stage::Preflight, Stage::BackingUp, None)
        .unwrap();
    let manifest = backup::prepare(&mut store, &tx, &f.0).unwrap();
    assert_eq!(store.transaction(id).unwrap().stage, Stage::Prepared);
    let (directory, _) = backup::load(&store, &store.transaction(id).unwrap()).unwrap();
    snapshot::materialize(
        &directory.join("package"),
        &manifest.package,
        &f.0.join("old-package"),
    )
    .unwrap();
    assert_eq!(
        fs::read(f.0.join("old-package/cli.js")).unwrap(),
        fs::read(package.join("cli.js")).unwrap()
    );
    assert!(manifest
        .config
        .entries
        .iter()
        .any(|e| e.path == "auth.json" && e.kind == "missing"));
    assert_eq!(
        fs::read(f.0.join(".codex/config.toml")).unwrap(),
        b"private fixture"
    );
}
