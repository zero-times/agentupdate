mod common;
use agentupdate::{backup, storage::model::Stage, updater::rollback};
use common::{fixture, fixture_at};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
};
#[test]
fn changed_config_requires_confirmation_preserves_latest_and_restores_missing_and_modes() {
    let mut f = fixture(true);
    fs::write(f.root.join(".codex/config.toml"), b"new fixture").unwrap();
    fs::write(
        f.root.join(".codex/auth.json"),
        b"new authentication fixture",
    )
    .unwrap();
    fs::write(f.root.join(".codex/unrelated.txt"), b"keep me").unwrap();
    assert!(
        rollback::preview(&f.store, &f.id, &f.root)
            .unwrap()
            .configuration_changed
    );
    assert!(rollback::execute(&mut f.store, &f.id, &f.context, false).is_err());
    assert_eq!(f.store.transaction(&f.id).unwrap().stage, Stage::Succeeded);
    let result = rollback::execute(&mut f.store, &f.id, &f.context, true).unwrap();
    assert_eq!(result.stage, Stage::Restored);
    assert_eq!(
        fs::read(f.root.join(".codex/config.toml")).unwrap(),
        b"original fixture"
    );
    assert!(!f.root.join(".codex/auth.json").exists());
    assert_eq!(
        fs::read(f.root.join(".codex/unrelated.txt")).unwrap(),
        b"keep me"
    );
    assert_eq!(
        fs::metadata(f.root.join(".codex/config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    assert_eq!(
        fs::metadata(f.root.join(".codex"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
    let (directory, _) = backup::load(&f.store, &result).unwrap();
    assert!(fs::read_dir(directory.join("rescue"))
        .unwrap()
        .next()
        .is_some());
}
#[test]
fn original_missing_directory_is_restored_and_failed_updates_can_recover() {
    let mut f = fixture_at(false, Stage::NeedsRecovery);
    fs::create_dir(f.root.join(".codex")).unwrap();
    fs::write(f.root.join(".codex/config.toml"), b"created after update").unwrap();
    assert_eq!(
        rollback::execute(&mut f.store, &f.id, &f.context, true)
            .unwrap()
            .stage,
        Stage::Restored
    );
    assert!(!f.root.join(".codex").exists());
}
#[test]
fn corrupt_material_or_external_config_link_never_reports_rollback_success() {
    let mut f = fixture(true);
    let tx = f.store.transaction(&f.id).unwrap();
    let (directory, _) = backup::load(&f.store, &tx).unwrap();
    let payload = fs::read_dir(directory.join("package"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(payload, b"damaged").unwrap();
    assert!(rollback::execute(&mut f.store, &f.id, &f.context, true).is_err());
    assert_ne!(f.store.transaction(&f.id).unwrap().stage, Stage::Restored);
    let mut other = fixture(true);
    fs::remove_file(other.root.join(".codex/config.toml")).unwrap();
    fs::write(other.root.join("outside"), b"outside").unwrap();
    symlink("../outside", other.root.join(".codex/config.toml")).unwrap();
    assert!(rollback::execute(&mut other.store, &other.id, &other.context, true).is_err());
    assert_eq!(fs::read(other.root.join("outside")).unwrap(), b"outside");
}
#[test]
fn conflicting_configuration_type_keeps_material_and_reports_recovery_failed() {
    let mut f = fixture(true);
    fs::create_dir(f.root.join(".codex/auth.json")).unwrap();
    fs::write(f.root.join(".codex/auth.json/unrelated"), b"must survive").unwrap();
    assert!(rollback::execute(&mut f.store, &f.id, &f.context, true).is_err());
    let tx = f.store.transaction(&f.id).unwrap();
    assert_eq!(tx.stage, Stage::RecoveryFailed);
    assert!(backup::load(&f.store, &tx).is_ok());
    assert_eq!(
        fs::read(f.root.join(".codex/auth.json/unrelated")).unwrap(),
        b"must survive"
    );
}
