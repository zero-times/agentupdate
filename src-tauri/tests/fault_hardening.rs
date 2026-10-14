mod common;
use agentupdate::{
    agents::discovery::{self, DiscoveryContext},
    backup::{self, snapshot},
    platform::process,
    storage::model::Stage,
    updater::{npm, recovery, rollback},
};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::Path,
    time::Duration,
};
#[test]
fn isolated_health_checks_can_resolve_macos_system_architecture_utilities() {
    let f = common::fixture(true);
    let tx = f.store.transaction(&f.id).unwrap();
    let launcher = tx.installation.resolved_command.clone();
    fs::write(&launcher, "#!/usr/bin/env node\nif(process.argv.includes('--help')) console.log(require('node:child_process').execFileSync('sysctl',['-n','hw.machine'],{encoding:'utf8'}));else console.log('1.0.0');\n").unwrap();
    let (version, _) = discovery::verify(
        &DiscoveryContext {
            home: f.root.clone(),
            directories: vec![],
        },
        tx.installation.node_path.as_ref().unwrap(),
        &launcher,
        true,
    )
    .unwrap();
    assert_eq!(version, "1.0.0");
}
#[test]
fn paths_with_spaces_node_flags_and_other_instances_have_conservative_occupancy_checks() {
    let f = common::fixture(true);
    let mut i = f.store.transaction(&f.id).unwrap().installation;
    i.command_path = "/fixture with spaces/bin/codex".into();
    i.resolved_command = "/fixture with spaces/lib/codex.js".into();
    i.package_root = Some("/fixture with spaces/lib".into());
    i.node_path = Some("/node directory/node".into());
    assert!(npm::process_uses_instance(
        "/fixture with spaces/bin/codex --version",
        &i
    ));
    assert!(npm::process_uses_instance(
        "/node directory/node --trace-warnings /fixture with spaces/lib/codex.js",
        &i
    ));
    assert!(!npm::process_uses_instance(
        "/different/bin/codex --version",
        &i
    ));
}
#[test]
fn unreadable_backup_wrong_version_and_bad_health_never_report_verified_success() {
    let f = common::fixture_at(true, Stage::Prepared);
    let tx = f.store.transaction(&f.id).unwrap();
    let package = tx.installation.package_root.as_ref().unwrap();
    fs::set_permissions(
        f.root.join(".codex/config.toml"),
        fs::Permissions::from_mode(0),
    )
    .unwrap();
    assert!(snapshot::scan(&f.root.join(".codex"), Some(&["config.toml"])).is_err());
    fs::set_permissions(
        f.root.join(".codex/config.toml"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    fs::write(
        package.join("cli.js"),
        "console.log(process.argv.includes('--help')?'fixture help':'9.9.9');",
    )
    .unwrap();
    assert!(npm::verify_installed(&tx, &f.context).is_err());
    fs::write(
        package.join("cli.js"),
        "if(process.argv.includes('--help'))process.exit(1);else console.log('1.0.0');",
    )
    .unwrap();
    assert!(discovery::verify(
        &DiscoveryContext {
            home: f.root.clone(),
            directories: vec![]
        },
        tx.installation.node_path.as_ref().unwrap(),
        &tx.installation.resolved_command,
        true
    )
    .is_err());
    assert_ne!(f.store.transaction(&f.id).unwrap().stage, Stage::Succeeded);
}
#[test]
fn changed_config_directory_permissions_are_confirmed_and_restore_original_mode() {
    let mut f = common::fixture(true);
    fs::set_permissions(f.root.join(".codex"), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        rollback::preview(&f.store, &f.id, &f.root)
            .unwrap()
            .configuration_changed
    );
    assert!(rollback::execute(&mut f.store, &f.id, &f.context, false).is_err());
    rollback::execute(&mut f.store, &f.id, &f.context, true).unwrap();
    assert_eq!(
        fs::metadata(f.root.join(".codex"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
}
#[test]
fn nested_managed_link_restores_missing_target_parent_without_capturing_siblings() {
    let f = common::fixture(true);
    let source = f.root.join("nested-config");
    fs::create_dir_all(source.join("private")).unwrap();
    fs::set_permissions(source.join("private"), fs::Permissions::from_mode(0o750)).unwrap();
    fs::write(source.join("private/target"), b"private fixture").unwrap();
    fs::write(source.join("private/unrelated"), b"not managed").unwrap();
    symlink("private/target", source.join("config.toml")).unwrap();
    let saved = snapshot::scan(&source, Some(&["config.toml"])).unwrap();
    assert!(saved
        .entries
        .iter()
        .any(|e| e.path == "private" && e.mode == 0o750));
    assert!(!saved.entries.iter().any(|e| e.path.ends_with("unrelated")));
    let archive = f.root.join("nested-archive");
    snapshot::copy(&source, &archive, &saved).unwrap();
    fs::remove_file(source.join("config.toml")).unwrap();
    fs::remove_dir_all(source.join("private")).unwrap();
    let current = snapshot::scan(&source, Some(&["config.toml", "private/target"])).unwrap();
    assert!(current.entries.iter().all(|e| e.kind == "missing"));
    let restored = f.root.join("nested-restored");
    snapshot::materialize(&archive, &saved, &restored).unwrap();
    assert_eq!(
        fs::read(restored.join("config.toml")).unwrap(),
        b"private fixture"
    );
    assert_eq!(
        fs::metadata(restored.join("private"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
}
#[test]
fn interrupted_preparation_is_not_retried_and_corrupt_recovery_material_is_reported() {
    let mut f = common::fixture_at(true, Stage::Prepared);
    let report = recovery::reconcile(&mut f.store, &f.root).unwrap();
    assert_eq!(report.items[0].stage, Stage::PreparationFailed);
    let tx = f.store.transaction(&f.id).unwrap();
    let (directory, _) = backup::load(&f.store, &tx).unwrap();
    fs::write(directory.join("manifest.json"), b"broken").unwrap();
    assert!(rollback::execute(&mut f.store, &f.id, &f.context, true).is_err());
    assert_eq!(
        f.store.transaction(&f.id).unwrap().stage,
        Stage::PreparationFailed
    );
}
#[test]
fn subprocess_timeout_and_output_limits_stop_the_entire_owned_group() {
    let f = common::fixture(true);
    let marker = f.root.join("child-pid");
    let script=format!("const c=require('node:child_process').spawn('/bin/sleep',['30']);require('node:fs').writeFileSync({},String(c.pid));setInterval(()=>{{}},1000)",serde_json::to_string(&marker).unwrap());
    let result = process::run(
        Path::new("/opt/homebrew/bin/node"),
        &["-e".into(), script],
        &f.root,
        &[],
        Duration::from_millis(300),
        1024,
    );
    assert_eq!(result.unwrap_err().code, "PROCESS_TIMEOUT");
    let pid = fs::read_to_string(marker).unwrap();
    let status = std::process::Command::new("/bin/ps")
        .args(["-p", pid.trim(), "-o", "stat="])
        .output()
        .unwrap();
    assert!(
        !status.status.success()
            || String::from_utf8_lossy(&status.stdout)
                .trim()
                .starts_with('Z')
    );
    let result = process::run(
        Path::new("/opt/homebrew/bin/node"),
        &[
            "-e".into(),
            "process.stdout.write('x'.repeat(10000));setInterval(()=>{},1000)".into(),
        ],
        &f.root,
        &[],
        Duration::from_secs(2),
        64,
    );
    assert_eq!(result.unwrap_err().code, "PROCESS_OUTPUT_LIMIT");
}
