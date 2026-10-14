use agentupdate::{
    agents::{discovery::Installation, versions::AvailableVersion, AgentId},
    platform::files,
    storage::{
        model::{PlannedItem, Stage},
        Store,
    },
};
use std::{fs, path::PathBuf};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("agentupdate-storage-{}", uuid::Uuid::new_v4())))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn item() -> PlannedItem {
    PlannedItem {
        installation: Installation {
            id: "codex-test".into(),
            agent_id: AgentId::Codex,
            command_path: "/test/bin/codex".into(),
            resolved_command: "/test/lib/cli.js".into(),
            package_root: Some("/test/lib".into()),
            prefix: Some("/test".into()),
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
            tarball: "https://registry.npmjs.org/@openai/codex/-/codex-1.1.0.tgz".into(),
            integrity: "sha512-test".into(),
        },
    }
}
fn prepared(store: &mut Store, transaction: &str) -> String {
    store
        .transition(transaction, Stage::Queued, Stage::Preflight, None)
        .unwrap();
    store
        .transition(transaction, Stage::Preflight, Stage::BackingUp, None)
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let partial = store.begin_restore_point(&id).unwrap();
    let header = serde_json::json!({"schemaVersion":1,"id":id,"transactionId":transaction,"installationId":"codex-test","originalVersion":"1.0.0","targetVersion":"1.1.0"});
    files::write_atomic(
        &partial.join("manifest.json"),
        &serde_json::to_vec(&header).unwrap(),
    )
    .unwrap();
    store.publish_restore_point(transaction, &id).unwrap();
    id
}
#[test]
fn reopen_preserves_installing_and_cas_rejects_stale_stages() {
    let fixture = Fixture::new();
    let mut store = Store::open(&fixture.0).unwrap();
    let plan = store.create_plan(vec![item()]).unwrap();
    let id = &plan.transactions[0].id;
    assert!(store.create_plan(vec![item()]).is_err());
    assert!(store
        .transition(id, Stage::Queued, Stage::Succeeded, None)
        .is_err());
    prepared(&mut store, id);
    store
        .transition(id, Stage::Prepared, Stage::Installing, None)
        .unwrap();
    assert!(store
        .transition(id, Stage::Prepared, Stage::Installing, None)
        .is_err());
    drop(store);
    let store = Store::open(&fixture.0).unwrap();
    assert_eq!(store.transaction(id).unwrap().stage, Stage::Installing);
    assert_eq!(
        store.plans().unwrap()[0].transactions[0].stage,
        Stage::Installing
    );
    assert_eq!(store.events(id).unwrap().last().unwrap().code, "installing");
}
#[test]
fn corrupt_missing_and_partial_material_never_allows_installation() {
    let fixture = Fixture::new();
    let mut store = Store::open(&fixture.0).unwrap();
    let id = store.create_plan(vec![item()]).unwrap().transactions[0]
        .id
        .clone();
    store
        .transition(&id, Stage::Queued, Stage::Preflight, None)
        .unwrap();
    store
        .transition(&id, Stage::Preflight, Stage::BackingUp, None)
        .unwrap();
    assert!(store
        .transition(&id, Stage::BackingUp, Stage::Prepared, None)
        .is_err());
    let point = uuid::Uuid::new_v4().to_string();
    store.begin_restore_point(&point).unwrap();
    assert!(store.publish_restore_point(&id, &point).is_err());
    assert_eq!(store.transaction(&id).unwrap().stage, Stage::BackingUp);
    // An orphan complete directory without a database reference is also unusable.
    fs::create_dir(
        store
            .root
            .join("restores")
            .join(uuid::Uuid::new_v4().to_string()),
    )
    .unwrap();
}
#[test]
fn manifest_identity_and_hash_protect_the_reference() {
    let fixture = Fixture::new();
    let mut store = Store::open(&fixture.0).unwrap();
    let id = store.create_plan(vec![item()]).unwrap().transactions[0]
        .id
        .clone();
    let point = prepared(&mut store, &id);
    store.verify_restore_point(&point).unwrap();
    fs::write(
        store
            .root
            .join("restores")
            .join(&point)
            .join("manifest.json"),
        b"{}",
    )
    .unwrap();
    assert!(store
        .transition(&id, Stage::Prepared, Stage::Installing, None)
        .is_err());
    assert_eq!(store.transaction(&id).unwrap().stage, Stage::Prepared);
}
#[test]
fn migration_newer_schema_corruption_and_record_disagreement_preserve_data() {
    let fixture = Fixture::new();
    let mut store = Store::open(&fixture.0).unwrap();
    let id = store.create_plan(vec![item()]).unwrap().transactions[0]
        .id
        .clone();
    drop(store);
    let db = fixture.0.join("state.sqlite3");
    let connection = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    connection
        .execute(
            "UPDATE transactions SET stage='succeeded' WHERE id=?1",
            [&id],
        )
        .unwrap();
    drop(connection);
    let store = Store::open(&fixture.0).unwrap();
    assert!(store.transaction(&id).is_err());
    drop(store);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("PRAGMA user_version=99;").unwrap();
    drop(connection);
    assert!(Store::open(&fixture.0).is_err());
    assert!(db.exists());
    let other = Fixture::new();
    fs::create_dir(&other.0).unwrap();
    let corrupt = other.0.join("state.sqlite3");
    fs::write(&corrupt, b"corrupt original").unwrap();
    assert!(Store::open(&other.0).is_err());
    assert_eq!(fs::read(corrupt).unwrap(), b"corrupt original");
}
