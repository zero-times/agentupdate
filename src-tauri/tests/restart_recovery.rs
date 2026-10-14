mod common;
use agentupdate::{
    platform::locks::OperationLock,
    storage::{model::Stage, Store},
    updater::{recovery, rollback},
};
use std::{fs, os::unix::process::ExitStatusExt, path::PathBuf, process::Command};
#[test]
#[ignore = "Child killed by restart recovery test"]
fn interrupted_child() {
    let phase = std::env::var("AGENTUPDATE_CRASH_PHASE").unwrap();
    let mut f = common::fixture_at(
        true,
        if phase == "installing" {
            Stage::Installing
        } else {
            Stage::Recovering
        },
    );
    let tx = f.store.transaction(&f.id).unwrap();
    f.store.set_plan_state(&tx.plan_id, "running").unwrap();
    let _global = OperationLock::acquire(&f.store.root.join("locks/operation.lock")).unwrap();
    let _instance = OperationLock::npm(tx.installation.prefix.as_ref().unwrap()).unwrap();
    // Reproduce an interrupted package publication with its original material already durable.
    fs::remove_dir_all(tx.installation.package_root.as_ref().unwrap()).unwrap();
    let marker = PathBuf::from(std::env::var_os("AGENTUPDATE_CRASH_MARKER").unwrap());
    fs::write(
        &marker,
        serde_json::to_vec(&serde_json::json!({"root":f.root,"id":f.id})).unwrap(),
    )
    .unwrap();
    fs::File::open(marker).unwrap().sync_all().unwrap();
    unsafe {
        libc::kill(libc::getpid(), libc::SIGKILL);
    }
    panic!("Child was not killed");
}
#[test]
fn killed_install_and_recovery_processes_are_detected_and_restore_offline() {
    let mut results = vec![];
    for phase in ["installing", "recovering"] {
        let marker =
            std::env::temp_dir().join(format!("agentupdate-crash-marker-{}", uuid::Uuid::new_v4()));
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "interrupted_child", "--ignored"])
            .env("AGENTUPDATE_CRASH_PHASE", phase)
            .env("AGENTUPDATE_CRASH_MARKER", &marker)
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
        let root = PathBuf::from(value["root"].as_str().unwrap());
        let id = value["id"].as_str().unwrap();
        let mut store = Store::open(&root.join("app-data")).unwrap();
        let before = store.transaction(id).unwrap();
        assert!(before.stage.incomplete_mutation());
        let report = recovery::reconcile(&mut store, &root).unwrap();
        assert!(!report.busy);
        assert_eq!(report.items.len(), 1);
        assert!(report.items[0].material_ready);
        assert!(report.items[0].installation_error.is_some());
        assert_eq!(
            store.transaction(id).unwrap().stage,
            if phase == "installing" {
                Stage::NeedsRecovery
            } else {
                Stage::RecoveryFailed
            }
        );
        assert_eq!(store.plan(&before.plan_id).unwrap().state, "paused");
        let context = agentupdate::updater::npm::Context {
            home: root.clone(),
            npm: PathBuf::new(),
            cache: None,
            offline: true,
        };
        let result = rollback::execute(&mut store, id, &context, false).unwrap();
        assert_eq!(result.stage, Stage::Restored);
        assert_eq!(
            fs::read(root.join(".codex/config.toml")).unwrap(),
            b"original fixture"
        );
        results.push(serde_json::json!({"phase":phase,"killedBySignal":9,"detectedAfterReopen":true,"noSuccessOrAutomaticInstall":true,"locksReleasedByKernel":true,"offlineRestoreVerified":true}));
        drop(store);
        fs::remove_dir_all(root).unwrap();
        fs::remove_file(marker).unwrap();
    }
    if let Some(path) = std::env::var_os("AGENTUPDATE_RECOVERY_EVIDENCE") {
        fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
}
#[test]
fn another_running_process_is_not_reclassified_as_a_crash() {
    let mut f = common::fixture_at(true, Stage::Installing);
    let lock = OperationLock::acquire(&f.store.root.join("locks/operation.lock")).unwrap();
    let result = recovery::reconcile(&mut f.store, &f.root).unwrap();
    assert!(result.busy);
    assert_eq!(f.store.transaction(&f.id).unwrap().stage, Stage::Installing);
    drop(lock);
}
