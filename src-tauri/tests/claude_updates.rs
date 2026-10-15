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
    updater::{self, native, npm, rollback},
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture {
    root: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn executable(path: &Path, version: &str) {
    let source = path.with_extension("c");
    fs::write(&source, format!("#include <stdio.h>\n#include <string.h>\nint main(int n,char**v){{puts(n>1 && !strcmp(v[1],\"--version\")?\"{version} (Claude Code)\":\"Claude help fixture\");return 0;}}\n")).unwrap();
    assert!(Command::new("/usr/bin/cc")
        .args(["-o"])
        .arg(path)
        .arg(&source)
        .status()
        .unwrap()
        .success());
    fs::remove_file(source).unwrap();
}
fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!("agentupdate-claude-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    Fixture { root }
}

#[test]
fn native_roundtrip_restores_global_config_credentials_absence_and_unrelated_files() {
    let f = fixture();
    let versions = f.root.join(".local/share/claude/versions");
    let bin = f.root.join(".local/bin");
    fs::create_dir_all(&versions).unwrap();
    fs::create_dir_all(&bin).unwrap();
    executable(&versions.join("1.0.0"), "1.0.0");
    symlink(versions.join("1.0.0"), bin.join("claude")).unwrap();
    fs::create_dir(f.root.join(".claude")).unwrap();
    fs::write(f.root.join(".claude/settings.json"), b"{\"fixture\":true}").unwrap();
    fs::set_permissions(
        f.root.join(".claude/settings.json"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    fs::write(f.root.join(".claude.json"), b"global original fixture").unwrap();
    fs::set_permissions(
        f.root.join(".claude.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let before = backup::snapshot::scan(&versions, None).unwrap();
    let installation = discover(
        &DiscoveryContext {
            home: f.root.clone(),
            directories: vec![bin.clone()],
        },
        &[],
    )
    .unwrap()
    .instances
    .into_iter()
    .find(|i| i.agent_id == AgentId::Claude)
    .unwrap();
    assert!(installation.supported);
    assert_eq!(installation.source, "user_native");
    let target_path = f.root.join("target");
    executable(&target_path, "1.1.0");
    let bytes = fs::read(&target_path).unwrap();
    let target = AvailableVersion {
        version: "1.1.0".into(),
        prerelease: false,
        tarball: format!(
            "https://downloads.claude.ai/claude-code-releases/1.1.0/darwin-{}/claude",
            if installation.architecture == "arm64" {
                "arm64"
            } else {
                "x64"
            }
        ),
        integrity: format!(
            "sha256-{}",
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&bytes))
        ),
    };
    let mut store = Store::open(&f.root.join("app-data")).unwrap();
    let plan = store
        .create_plan(vec![PlannedItem {
            installation,
            target,
        }])
        .unwrap();
    let id = &plan.transactions[0].id;
    let work = store.root.join("work").join(id);
    fs::create_dir_all(&work).unwrap();
    fs::set_permissions(&work, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(work.join("target.tgz"), bytes).unwrap();
    fs::set_permissions(work.join("target.tgz"), fs::Permissions::from_mode(0o600)).unwrap();
    let ctx = npm::Context {
        home: f.root.clone(),
        npm: PathBuf::new(),
        cache: None,
        offline: true,
    };
    assert_eq!(
        updater::execute_item(&mut store, id, &ctx).unwrap().stage,
        Stage::Succeeded
    );
    fs::write(f.root.join(".claude/settings.json"), b"changed fixture").unwrap();
    fs::write(f.root.join(".claude.json"), b"global changed fixture").unwrap();
    fs::write(
        f.root.join(".claude/.credentials.json"),
        b"synthetic credential fixture",
    )
    .unwrap();
    fs::write(f.root.join(".claude/unrelated.txt"), b"keep fixture").unwrap();
    assert!(
        rollback::preview(&store, id, &f.root)
            .unwrap()
            .configuration_changed
    );
    assert_eq!(
        rollback::execute(&mut store, id, &ctx, true).unwrap().stage,
        Stage::Restored
    );
    assert_eq!(
        backup::snapshot::scan(&versions, None).unwrap().entries,
        before.entries
    );
    assert_eq!(
        fs::read(f.root.join(".claude.json")).unwrap(),
        b"global original fixture"
    );
    assert_eq!(
        fs::read(f.root.join(".claude/settings.json")).unwrap(),
        b"{\"fixture\":true}"
    );
    assert!(!f.root.join(".claude/.credentials.json").exists());
    assert!(f.root.join(".claude/unrelated.txt").is_file());
}

#[test]
fn native_manifest_and_cask_only_allow_verified_architecture_artifacts() {
    let mut value = serde_json::json!({"version":"2.1.296","platforms":{"darwin-arm64":{"binary":"claude","checksum":"a".repeat(64)}}});
    assert!(native::from_manifest(&value, "2.1.296", "arm64").is_ok());
    assert!(native::from_manifest(&value, "2.1.296", "x64").is_err());
    assert!(native::from_manifest(&value, "2.1.297", "arm64").is_err());
    value["platforms"]["darwin-arm64"]["checksum"] = "invalid".into();
    assert!(native::from_manifest(&value, "2.1.296", "arm64").is_err());
    let mut value = serde_json::json!({"token":"claude-code","version":"2.1.287","url":"https://downloads.claude.ai/claude-code-releases/2.1.287/darwin-arm64/claude","sha256":"b".repeat(64),"artifacts":[{"binary":["claude"]}]});
    assert!(native::from_cask(&value, "arm64").is_ok());
    value["artifacts"][0]["target"] = "$HOMEBREW_PREFIX/bin/unrelated".into();
    assert!(native::from_cask(&value, "arm64").is_err());
    value["artifacts"][0]["target"] = "$HOMEBREW_PREFIX/bin/claude".into();
    assert!(native::from_cask(&value, "arm64").is_ok());
    value["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"binary":["unrelated"]}));
    assert!(native::from_cask(&value, "arm64").is_err());
    value["artifacts"].as_array_mut().unwrap().pop();
    value["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"preflight":"unexpected lifecycle"}));
    assert!(native::from_cask(&value, "arm64").is_err());
}

#[test]
#[ignore = "Two actual official native versions in isolated HOME; requires cached verified binaries"]
fn actual_native_upgrade_and_rollback() {
    let f = fixture();
    let versions = f.root.join(".local/share/claude/versions");
    let bin = f.root.join(".local/bin");
    fs::create_dir_all(&versions).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let old = PathBuf::from(std::env::var_os("AGENTUPDATE_CLAUDE_OLD").unwrap());
    let new = PathBuf::from(std::env::var_os("AGENTUPDATE_CLAUDE_NEW").unwrap());
    fs::copy(old, versions.join("2.1.286")).unwrap();
    fs::set_permissions(versions.join("2.1.286"), fs::Permissions::from_mode(0o755)).unwrap();
    symlink(versions.join("2.1.286"), bin.join("claude")).unwrap();
    let installation = discover(
        &DiscoveryContext {
            home: f.root.clone(),
            directories: vec![bin],
        },
        &[],
    )
    .unwrap()
    .instances
    .into_iter()
    .find(|i| i.agent_id == AgentId::Claude)
    .unwrap();
    assert!(installation.supported);
    assert_eq!(installation.version.as_deref(), Some("2.1.286"));
    let bytes = fs::read(new).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        "6eab8333fe2121553100d8f40bfada384a3e989b94f947e18ba6677a6fcb41ea"
    );
    let target = AvailableVersion {
        version: "2.1.287".into(),
        prerelease: false,
        tarball: "https://downloads.claude.ai/claude-code-releases/2.1.287/darwin-arm64/claude"
            .into(),
        integrity: format!(
            "sha256-{}",
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&bytes))
        ),
    };
    let mut store = Store::open(&f.root.join("app-data")).unwrap();
    let plan = store
        .create_plan(vec![PlannedItem {
            installation,
            target,
        }])
        .unwrap();
    let id = &plan.transactions[0].id;
    let work = store.root.join("work").join(id);
    fs::create_dir_all(&work).unwrap();
    fs::set_permissions(&work, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(work.join("target.tgz"), bytes).unwrap();
    fs::set_permissions(work.join("target.tgz"), fs::Permissions::from_mode(0o600)).unwrap();
    let ctx = npm::Context {
        home: f.root.clone(),
        npm: PathBuf::new(),
        cache: None,
        offline: true,
    };
    let before = backup::snapshot::scan(&versions, None).unwrap();
    let updated = updater::execute_item(&mut store, id, &ctx).unwrap();
    assert_eq!(updated.stage, Stage::Succeeded);
    fs::create_dir(f.root.join(".claude")).unwrap();
    fs::write(
        f.root.join(".claude/settings.json"),
        b"synthetic changed settings",
    )
    .unwrap();
    let restored = rollback::execute(&mut store, id, &ctx, true).unwrap();
    assert_eq!(restored.stage, Stage::Restored);
    assert!(!f.root.join(".claude").exists());
    assert_eq!(
        backup::snapshot::scan(&versions, None).unwrap().entries,
        before.entries
    );
    if let Some(path) = std::env::var_os("AGENTUPDATE_CLAUDE_EVIDENCE") {
        fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","original":"2.1.286","target":"2.1.287","stage":restored.stage,"architecture":"arm64","execution":"native Apple Silicon","installationTreeExact":true,"originalMissingConfigDirectoryRestored":true,"isolation":"Temporary HOME; actual official binaries; no daily installation or credential changes"})).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "Read-only live Homebrew API refresh; requires a real standard Claude cask installation"]
fn live_homebrew_preparation_refreshes_catalog_without_changing_installation() {
    let f = fixture();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let command = PathBuf::from(std::env::var_os("AGENTUPDATE_LIVE_CLAUDE_COMMAND").unwrap());
    let installation = discover(
        &DiscoveryContext {
            home: home.clone(),
            directories: vec![command.parent().unwrap().into()],
        },
        &[],
    )
    .unwrap()
    .instances
    .into_iter()
    .find(|i| i.agent_id == AgentId::Claude && i.command_path == command)
    .unwrap();
    assert!(installation.supported);
    assert_eq!(installation.source, "homebrew");
    let original_link = fs::read_link(&command).unwrap();
    let original_hash = Sha256::digest(fs::read(&installation.resolved_command).unwrap());
    let target = native::catalog(&installation).unwrap().versions.remove(0);
    let mut store = Store::open(&f.root.join("app-data")).unwrap();
    let plan = store
        .create_plan(vec![PlannedItem {
            installation: installation.clone(),
            target,
        }])
        .unwrap();
    let tx = &plan.transactions[0];
    let ctx = native::context(&installation).unwrap();
    native::check_homebrew_target(tx, &ctx, true).unwrap();
    native::check_homebrew_target(tx, &ctx, false).unwrap();
    assert_eq!(fs::read_link(&command).unwrap(), original_link);
    assert_eq!(
        Sha256::digest(fs::read(&installation.resolved_command).unwrap()),
        original_hash
    );
}
