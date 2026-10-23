use agentupdate::{
    agents::{discovery::Installation, versions::AvailableVersion, AgentId},
    platform::{files, locks::OperationLock},
    storage::{
        model::{PlannedItem, Stage},
        Store,
    },
    updater::queue,
};
use std::{fs, path::PathBuf, process::Command};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("agentupdate-queue-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn item(agent: AgentId) -> PlannedItem {
    PlannedItem {
        installation: Installation {
            id: agent.key().into(),
            agent_id: agent,
            command_path: format!("/fixture/{}", agent.key()).into(),
            resolved_command: "/fixture/package/cli.js".into(),
            package_root: Some("/fixture/package".into()),
            prefix: Some("/fixture".into()),
            node_path: Some("/node".into()),
            version: Some("1.0.0".into()),
            architecture: "arm64".into(),
            source: "user_npm".into(),
            supported: true,
            version_verified: true,
            reason: None,
        },
        target: AvailableVersion {
            version: "1.1.0".into(),
            prerelease: false,
            tarball: String::new(),
            integrity: String::new(),
        },
    }
}
fn success(store: &mut Store, id: &str) -> agentupdate::storage::model::UpdateTransaction {
    store
        .transition(id, Stage::Queued, Stage::Preflight, None)
        .unwrap();
    let tx = store
        .transition(id, Stage::Preflight, Stage::BackingUp, None)
        .unwrap();
    let point = uuid::Uuid::new_v4().to_string();
    let directory = store.begin_restore_point(&point).unwrap();
    files::write_atomic(&directory.join("manifest.json"),&serde_json::to_vec(&serde_json::json!({"schemaVersion":1,"id":point,"transactionId":id,"installationId":tx.installation.id,"originalVersion":"1.0.0","targetVersion":"1.1.0"})).unwrap()).unwrap();
    store.publish_restore_point(id, &point).unwrap();
    store
        .transition(id, Stage::Prepared, Stage::Installing, None)
        .unwrap();
    store
        .transition(id, Stage::Installing, Stage::Verifying, None)
        .unwrap();
    store
        .transition(id, Stage::Verifying, Stage::Succeeded, None)
        .unwrap()
}
#[test]
fn failure_pauses_and_explicit_continue_keeps_successful_items() {
    let f = Fixture::new();
    let mut store = Store::open(&f.0).unwrap();
    let mut third = item(AgentId::Codex);
    third.installation.id = "codex-second-instance".into();
    let plan = store
        .create_plan(vec![item(AgentId::Codex), item(AgentId::Gemini), third])
        .unwrap();
    let mut calls = vec![];
    let result = queue::run(&mut store, &plan.id, false, |s, id| {
        calls.push(id.to_string());
        if calls.len() == 2 {
            s.transition(id, Stage::Queued, Stage::Preflight, None)?;
            s.transition(
                id,
                Stage::Preflight,
                Stage::PreparationFailed,
                Some("NETWORK"),
            )
        } else {
            Ok(success(s, id))
        }
    })
    .unwrap();
    assert_eq!(result.state, "paused");
    assert_eq!(calls.len(), 2);
    assert_eq!(result.transactions[0].stage, Stage::Succeeded);
    assert_eq!(result.transactions[2].stage, Stage::Queued);
    assert!(queue::run(&mut store, &plan.id, false, |_, _| panic!(
        "Must not execute without continuation"
    ))
    .is_err());
    let continued = queue::run(&mut store, &plan.id, true, |s, id| Ok(success(s, id))).unwrap();
    assert_eq!(continued.state, "finished");
    assert_eq!(continued.transactions[1].stage, Stage::PreparationFailed);
    assert_eq!(continued.transactions[2].stage, Stage::Succeeded);
}
#[test]
fn cancellation_during_mutation_preserves_current_result_and_cancels_remaining() {
    let f = Fixture::new();
    let mut store = Store::open(&f.0).unwrap();
    let plan = store
        .create_plan(vec![item(AgentId::Codex), item(AgentId::Gemini)])
        .unwrap();
    let result = queue::run(&mut store, &plan.id, false, |s, id| {
        let result = success(s, id);
        s.request_cancel(&plan.id)?;
        Ok(result)
    })
    .unwrap();
    assert!(result.cancel_requested);
    assert_eq!(result.state, "cancelled");
    assert_eq!(result.transactions[0].stage, Stage::Succeeded);
    assert_eq!(result.transactions[1].stage, Stage::Cancelled);
    drop(store);
    let store = Store::open(&f.0).unwrap();
    assert!(store.cancel_requested(&plan.id).unwrap());
}
#[test]
fn locks_exclude_another_process_and_release_on_drop() {
    let f = Fixture::new();
    fs::create_dir(&f.0).unwrap();
    let path = f.0.join("operation.lock");
    let lock = OperationLock::acquire(&path).unwrap();
    assert!(OperationLock::acquire(&path).is_err());
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_lock_probe", "--ignored"])
        .env("AGENTUPDATE_LOCK_FIXTURE", &path)
        .status()
        .unwrap();
    assert!(status.success());
    drop(lock);
    assert!(OperationLock::acquire(&path).is_ok());
}
#[test]
#[ignore = "Invoked by parent with a temporary lock path"]
fn child_lock_probe() {
    let path = PathBuf::from(std::env::var_os("AGENTUPDATE_LOCK_FIXTURE").unwrap());
    assert!(OperationLock::acquire(&path).is_err());
}
#[test]
fn queued_cancellation_survives_schema_one_migration() {
    let f = Fixture::new();
    let mut store = Store::open(&f.0).unwrap();
    let plan = store.create_plan(vec![item(AgentId::Codex)]).unwrap();
    drop(store);
    let db = rusqlite::Connection::open(f.0.join("state.sqlite3")).unwrap();
    db.execute_batch("ALTER TABLE plans DROP COLUMN cancel_requested; PRAGMA user_version=1;")
        .unwrap();
    drop(db);
    let mut store = Store::open(&f.0).unwrap();
    assert!(!store.cancel_requested(&plan.id).unwrap());
    let result = store.request_cancel(&plan.id).unwrap();
    assert_eq!(result.transactions[0].stage, Stage::Cancelled);
    assert_eq!(result.state, "cancelled");
}

#[test]
fn concurrent_queue_overlaps_independent_tools_and_preserves_progress_targets() {
    use agentupdate::progress;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    let f = Fixture::new();
    let plan = Store::open(&f.0)
        .unwrap()
        .create_plan(vec![item(AgentId::Codex), item(AgentId::Gemini)])
        .unwrap();
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let reports = Arc::new(Mutex::new(Vec::new()));
    let output = reports.clone();
    let reporter: progress::Reporter = Arc::new(move |p| output.lock().unwrap().push(p));
    let result = queue::run_concurrent(&f.0, &plan.id, |root, id| {
        progress::with_reporter(Some(reporter.clone()), || {
            let mut store = Store::open(root)?;
            let target = store.transaction(id)?.installation.id;
            progress::operation(&target, || {
                let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                // A bounded wait fails the overlap assertion instead of deadlocking a test.
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                while active.load(Ordering::SeqCst) < 2
                    && peak.load(Ordering::SeqCst) < 2
                    && std::time::Instant::now() < deadline
                {
                    std::thread::yield_now();
                }
                let result = success(&mut store, id);
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(result)
            })
        })
    })
    .unwrap();
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    assert_eq!(result.state, "finished");
    assert!(result
        .transactions
        .iter()
        .all(|tx| tx.stage == Stage::Succeeded));
    let reports = reports.lock().unwrap();
    for target in ["codex", "gemini"] {
        assert!(reports
            .iter()
            .any(|p| p.target == target && p.phase == progress::Phase::Succeeded));
    }
}

#[test]
fn concurrent_failure_does_not_skip_other_items_and_failed_instance_can_retry() {
    let f = Fixture::new();
    let mut store = Store::open(&f.0).unwrap();
    let plan = store
        .create_plan(vec![
            item(AgentId::Codex),
            item(AgentId::Gemini),
            item(AgentId::Claude),
        ])
        .unwrap();
    let result = queue::run_concurrent(&f.0, &plan.id, |root, id| {
        let mut store = Store::open(root)?;
        if store.transaction(id)?.installation.agent_id == AgentId::Codex {
            // Simulates a failure before the adapter could transition its item.
            return Err(agentupdate::error::AppError::new("NETWORK", "fixture"));
        }
        Ok(success(&mut store, id))
    })
    .unwrap();
    assert_eq!(result.state, "finished");
    assert_eq!(result.transactions[0].stage, Stage::PreparationFailed);
    assert_eq!(
        result.transactions[0].failure_code.as_deref(),
        Some("NETWORK")
    );
    assert!(result.transactions[1..]
        .iter()
        .all(|tx| tx.stage == Stage::Succeeded));
    let tx = &result.transactions[1];
    // Simulate an old mutation failure. The current scan may start a fresh upgrade.
    store
        .transition(&tx.id, Stage::Succeeded, Stage::Recovering, None)
        .unwrap();
    store
        .transition(
            &tx.id,
            Stage::Recovering,
            Stage::RecoveryFailed,
            Some("FIXTURE"),
        )
        .unwrap();
    assert!(store.create_plan(vec![item(AgentId::Gemini)]).is_ok());
}

#[test]
fn concurrent_queue_serializes_same_agent_and_cancels_waiting_items() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = Fixture::new();
    let mut second = item(AgentId::Codex);
    second.installation.id = "codex-other-prefix".into();
    let plan = Store::open(&f.0)
        .unwrap()
        .create_plan(vec![item(AgentId::Codex), second])
        .unwrap();
    let calls = AtomicUsize::new(0);
    let result = queue::run_concurrent(&f.0, &plan.id, |root, id| {
        calls.fetch_add(1, Ordering::SeqCst);
        let mut store = Store::open(root)?;
        let result = success(&mut store, id);
        store.request_cancel(&plan.id)?;
        Ok(result)
    })
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.state, "cancelled");
    assert_eq!(
        result
            .transactions
            .iter()
            .filter(|tx| tx.stage == Stage::Succeeded)
            .count(),
        1
    );
    assert_eq!(
        result
            .transactions
            .iter()
            .filter(|tx| tx.stage == Stage::Cancelled)
            .count(),
        1
    );
}

#[test]
fn activity_guards_allow_other_tools_and_exclude_self_update_exit_and_reconciliation() {
    use agentupdate::service::{self, AppState};
    let f = Fixture::new();
    let _store = Store::open(&f.0).unwrap();
    let state = AppState::default();
    let first = service::begin_operation(&state, &f.0, false, Some("app:codex")).unwrap();
    let second = service::begin_operation(&state, &f.0, false, Some("app:claude")).unwrap();
    assert!(service::begin_operation(&state, &f.0, false, Some("app:codex")).is_err());
    assert!(service::begin_operation(&state, &f.0, true, Some("app:agentupdate")).is_err());
    assert!(state.operation.try_write().is_err());
    // Another process has a separate in-memory state but still observes the global file guard.
    let other = AppState::default();
    assert!(service::begin_operation(&other, &f.0, true, None).is_err());
    drop(first);
    drop(second);
    let exclusive = service::begin_operation(&state, &f.0, true, Some("app:agentupdate")).unwrap();
    assert!(service::begin_operation(&other, &f.0, false, Some("app:claude")).is_err());
    drop(exclusive);
    assert!(service::begin_operation(&other, &f.0, false, Some("app:claude")).is_ok());
}

#[test]
fn shared_installation_prefix_waits_for_the_current_update_instead_of_failing() {
    use std::{sync::mpsc, time::Duration};
    let f = Fixture::new();
    fs::create_dir(&f.0).unwrap();
    let prefix = fs::canonicalize(&f.0).unwrap();
    let held = OperationLock::npm(&prefix).unwrap();
    let (started, ready) = mpsc::channel();
    let (acquired, done) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        started.send(()).unwrap();
        let _lock = OperationLock::npm_wait(&prefix).unwrap();
        acquired.send(()).unwrap();
    });
    ready.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(matches!(
        done.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    drop(held);
    done.recv_timeout(Duration::from_secs(3)).unwrap();
    worker.join().unwrap();
}
