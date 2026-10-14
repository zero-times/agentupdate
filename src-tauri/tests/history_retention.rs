mod common;
use agentupdate::{
    backup,
    platform::locks::OperationLock,
    storage::{
        history::{self, ExportKind},
        model::{PlannedItem, Stage},
    },
};
use std::{fs, os::unix::fs::PermissionsExt};
#[test]
fn diagnostics_omit_raw_configuration_paths_process_output_and_unknown_codes() {
    let f = common::fixture(true);
    let secret = "fixture-super-secret-token-123";
    fs::write(f.root.join(".codex/auth.json"), secret).unwrap();
    let db = rusqlite::Connection::open(f.store.root.join("state.sqlite3")).unwrap();
    db.execute(
        "INSERT INTO events(transaction_id,code,created_at)VALUES(?1,?2,1)",
        rusqlite::params![f.id, secret],
    )
    .unwrap();
    db.execute(
        "INSERT INTO events(transaction_id,code,created_at)VALUES(?1,'SECRETALPHATOKEN',1)",
        [&f.id],
    )
    .unwrap();
    drop(db);
    let export = history::export(&f.store, &f.id, ExportKind::Diagnostics, false).unwrap();
    let bytes = fs::read(export.directory.join("diagnostics.json")).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains(secret));
    assert!(!text.contains("SECRETALPHATOKEN"));
    assert!(!text.contains(f.root.to_str().unwrap()));
    assert!(text.contains("REDACTED"));
    assert!(!export.sensitive);
    assert_eq!(
        fs::metadata(export.directory.join("diagnostics.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
#[test]
fn full_export_requires_explicit_sensitive_confirmation_and_matches_original_material() {
    let f = common::fixture(true);
    assert!(history::export(&f.store, &f.id, ExportKind::Restore, false).is_err());
    let export = history::export(&f.store, &f.id, ExportKind::Restore, true).unwrap();
    assert!(export.sensitive);
    let tx = f.store.transaction(&f.id).unwrap();
    let (original, m) = backup::load(&f.store, &tx).unwrap();
    let dest = export.directory.join("restore");
    backup::verify_material(&dest, &m).unwrap();
    assert_eq!(
        fs::read(original.join("manifest.json")).unwrap(),
        fs::read(dest.join("manifest.json")).unwrap()
    );
    assert_eq!(
        fs::metadata(&export.directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert!(history::reveal_export(&f.store, "../outside").is_err());
}
#[test]
fn latest_usable_restore_and_active_transactions_are_protected_while_old_cleanup_keeps_history() {
    let mut f = common::fixture(true);
    assert!(!history::details(&f.store, &f.id).unwrap().can_delete);
    assert!(history::cleanup(&mut f.store, &f.id).is_err());
    let original = f.store.transaction(&f.id).unwrap();
    let mut installation = original.installation.clone();
    installation.version = Some("1.1.0".into());
    let mut target = original.target.clone();
    target.version = "1.2.0".into();
    let next = f
        .store
        .create_plan(vec![PlannedItem {
            installation,
            target,
        }])
        .unwrap();
    let id = &next.transactions[0].id;
    f.store
        .transition(id, Stage::Queued, Stage::Preflight, None)
        .unwrap();
    let tx = f
        .store
        .transition(id, Stage::Preflight, Stage::BackingUp, None)
        .unwrap();
    backup::prepare(&mut f.store, &tx, &f.root).unwrap();
    assert!(!history::details(&f.store, id).unwrap().can_delete);
    assert!(history::cleanup(&mut f.store, id).is_err());
    f.store
        .transition(id, Stage::Prepared, Stage::Installing, None)
        .unwrap();
    f.store
        .transition(id, Stage::Installing, Stage::Verifying, None)
        .unwrap();
    f.store
        .transition(id, Stage::Verifying, Stage::Succeeded, None)
        .unwrap();
    assert!(history::details(&f.store, &f.id).unwrap().can_delete);
    let (path, _) = backup::load(&f.store, &original).unwrap();
    history::cleanup(&mut f.store, &f.id).unwrap();
    assert!(!path.exists());
    assert!(f
        .store
        .transaction(&f.id)
        .unwrap()
        .restore_point_id
        .is_none());
    assert_eq!(f.store.transaction(&f.id).unwrap().stage, Stage::Succeeded);
    assert!(backup::load(&f.store, &f.store.transaction(id).unwrap()).is_ok());
    assert!(history::cleanup(&mut f.store, id).is_err());
    assert_eq!(
        f.store.events(&f.id).unwrap().last().unwrap().code,
        "restore_deleted"
    );
}
#[test]
fn export_and_cleanup_refuse_to_compete_with_a_running_operation() {
    let mut f = common::fixture(true);
    let _lock = OperationLock::acquire(&f.store.root.join("locks/operation.lock")).unwrap();
    assert!(history::export(&f.store, &f.id, ExportKind::Diagnostics, false).is_err());
    assert!(history::cleanup(&mut f.store, &f.id).is_err());
}
