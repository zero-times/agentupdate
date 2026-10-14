use agentupdate::{
    agents::{
        discovery::{DiscoveryContext, Installation},
        versions, AgentId,
    },
    storage::{
        model::{PlannedItem, Stage},
        Store,
    },
    updater::{self, npm},
};
use base64::Engine;
use sha2::{Digest, Sha512};
use std::{fs, path::PathBuf};
fn instance() -> Installation {
    Installation {
        id: "test".into(),
        agent_id: AgentId::Codex,
        command_path: "/prefix/bin/codex".into(),
        resolved_command: "/prefix/lib/node_modules/@openai/codex/bin/codex.js".into(),
        package_root: Some("/prefix/lib/node_modules/@openai/codex".into()),
        prefix: Some("/prefix".into()),
        node_path: Some("/node".into()),
        version: Some("1.0.0".into()),
        architecture: "arm64".into(),
        source: "user_npm".into(),
        supported: true,
        version_verified: true,
        reason: None,
    }
}
#[test]
fn checksum_official_source_and_instance_process_checks_reject_invalid_inputs() {
    let bytes = b"verified artifact";
    let integrity = format!(
        "sha512-{}",
        base64::engine::general_purpose::STANDARD.encode(Sha512::digest(bytes))
    );
    npm::verify_integrity(bytes, &integrity).unwrap();
    assert!(npm::verify_integrity(b"broken", &integrity).is_err());
    assert!(npm::verify_integrity(bytes, "sha512-bad").is_err());
    let i = instance();
    assert!(npm::process_uses_instance(
        "node /prefix/bin/codex --version",
        &i
    ));
    assert!(npm::process_uses_instance("/prefix/bin/codex", &i));
    assert!(!npm::process_uses_instance(
        "/Applications/Codex.app/Contents/Resources/codex app-server",
        &i
    ));
    assert!(!npm::process_uses_instance(
        "/bin/zsh -c node /prefix/bin/codex",
        &i
    ));
    let target = versions::AvailableVersion {
        version: "1.1.0".into(),
        prerelease: false,
        tarball: "https://example.com/codex.tgz".into(),
        integrity,
    };
    assert!(npm::validate_source(&i, &target).is_err());
}
#[test]
#[ignore = "Requires isolated real npm fixtures and verified official cached artifacts"]
fn real_single_updates() {
    real_workflow(false);
}
#[test]
#[ignore = "Requires isolated real npm fixtures and verified official cached artifacts"]
fn real_roundtrip() {
    real_workflow(true);
}
fn real_workflow(roundtrip: bool) {
    let root = PathBuf::from(std::env::var_os("AGENTUPDATE_DISCOVERY_FIXTURE").unwrap());
    let context = DiscoveryContext {
        home: root.clone(),
        directories: vec![root.join("codex/bin"), root.join("gemini/bin")],
    };
    let report = agentupdate::agents::discovery::discover(&context, &[]).unwrap();
    assert_eq!(report.instances.len(), 2);
    let mut results = Vec::new();
    let mut store = Store::open(&root.join("app-data")).unwrap();
    for installation in report.instances {
        let target_version = match installation.agent_id {
            AgentId::Codex => "0.162.1",
            AgentId::Gemini => "0.63.0",
            AgentId::Claude | AgentId::Deepseek => {
                unreachable!("this fixture covers Codex and Gemini")
            }
        };
        let target: versions::AvailableVersion = serde_json::from_slice(
            &fs::read(
                root.join("targets")
                    .join(format!("{}.json", installation.agent_id.key())),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(target.version, target_version);
        let mut context = npm::Context::current(&installation).unwrap();
        context.home = root.clone();
        context.cache = Some(PathBuf::from(
            std::env::var_os("AGENTUPDATE_NPM_CACHE").unwrap(),
        ));
        context.offline = true;
        let before_package =
            agentupdate::backup::snapshot::scan(installation.package_root.as_ref().unwrap(), None)
                .unwrap();
        let mut invalid = target.clone();
        invalid.tarball = "https://example.com/untrusted.tgz".into();
        let rejected_plan = store
            .create_plan(vec![PlannedItem {
                installation: installation.clone(),
                target: invalid,
            }])
            .unwrap();
        let rejected_id = &rejected_plan.transactions[0].id;
        assert!(updater::execute_item(&mut store, rejected_id, &context).is_err());
        assert_eq!(
            store.transaction(rejected_id).unwrap().stage,
            Stage::PreparationFailed
        );
        assert_eq!(
            agentupdate::backup::snapshot::scan(installation.package_root.as_ref().unwrap(), None)
                .unwrap()
                .entries,
            before_package.entries
        );
        let plan = store
            .create_plan(vec![PlannedItem {
                installation,
                target,
            }])
            .unwrap();
        let id = &plan.transactions[0].id;
        let work = store.root.join("work").join(id);
        fs::create_dir_all(&work).unwrap();
        fs::copy(
            root.join("targets").join(format!(
                "{}.tgz",
                plan.transactions[0].installation.agent_id.key()
            )),
            work.join("target.tgz"),
        )
        .unwrap();
        let result = updater::execute_item(&mut store, id, &context);
        assert!(
            result.is_ok(),
            "update failed: {:?}, phase: {:?}",
            result,
            store.transaction(id).unwrap().stage
        );
        assert_eq!(result.unwrap().stage, Stage::Succeeded);
        let tx = store.transaction(id).unwrap();
        results.push(serde_json::json!({"agent":tx.installation.agent_id,"original":tx.installation.version,"target":tx.target.version,"stage":tx.stage,"restorePointVerified":agentupdate::backup::load(&store,&tx).is_ok(),"rejectedPreparationPreservedOriginal":true}));
        if roundtrip {
            let config = agentupdate::backup::config_root(&root, tx.installation.agent_id).unwrap();
            let names = agentupdate::backup::config_names(tx.installation.agent_id);
            let original_config = fs::read(config.join(names[0])).unwrap();
            fs::write(config.join(names[0]), b"changed fixture configuration").unwrap();
            fs::write(config.join(names[1]), b"new fixture credentials").unwrap();
            fs::write(config.join("unrelated.txt"), b"keep unrelated").unwrap();
            assert!(updater::rollback::execute(&mut store, id, &context, false).is_err());
            assert_eq!(store.transaction(id).unwrap().stage, Stage::Succeeded);
            let restored = updater::rollback::execute(&mut store, id, &context, true).unwrap();
            assert_eq!(restored.stage, Stage::Restored);
            assert_eq!(
                agentupdate::backup::snapshot::scan(
                    restored.installation.package_root.as_ref().unwrap(),
                    None
                )
                .unwrap()
                .entries,
                before_package.entries
            );
            assert_eq!(fs::read(config.join(names[0])).unwrap(), original_config);
            assert!(!config.join(names[1]).exists());
            assert_eq!(
                fs::read(config.join("unrelated.txt")).unwrap(),
                b"keep unrelated"
            );
            let (material, _) = agentupdate::backup::load(&store, &restored).unwrap();
            assert!(fs::read_dir(material.join("rescue"))
                .unwrap()
                .next()
                .is_some());
            results.last_mut().unwrap()["rollback"] = serde_json::json!({"stage":restored.stage,"originalPackageExact":true,"configurationRestored":true,"changedConfigurationPreserved":true,"unrelatedFilePreserved":true});
        }
    }
    if let Some(destination) = std::env::var_os("AGENTUPDATE_UPDATE_EVIDENCE") {
        fs::write(destination, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "Requires isolated real npm fixtures and verified official cached artifacts"]
fn real_batch() {
    use agentupdate::agents::discovery::discover;
    let root = PathBuf::from(std::env::var_os("AGENTUPDATE_DISCOVERY_FIXTURE").unwrap());
    let report = discover(
        &DiscoveryContext {
            home: root.clone(),
            directories: vec![root.join("codex/bin"), root.join("gemini/bin")],
        },
        &[],
    )
    .unwrap();
    let mut items = vec![];
    for agent in [AgentId::Codex, AgentId::Gemini] {
        let installation = report
            .instances
            .iter()
            .find(|i| i.agent_id == agent)
            .unwrap()
            .clone();
        let mut target: versions::AvailableVersion = serde_json::from_slice(
            &fs::read(root.join("targets").join(format!("{}.json", agent.key()))).unwrap(),
        )
        .unwrap();
        if agent == AgentId::Gemini {
            target.tarball = "https://example.com/untrusted.tgz".into();
        }
        items.push(PlannedItem {
            installation,
            target,
        });
    }
    let mut store = Store::open(&root.join("app-data")).unwrap();
    let plan = store.create_plan(items).unwrap();
    let contexts = |tx: &agentupdate::storage::model::UpdateTransaction| {
        let mut context = npm::Context::current(&tx.installation).unwrap();
        context.home = root.clone();
        context.cache = Some(PathBuf::from(
            std::env::var_os("AGENTUPDATE_NPM_CACHE").unwrap(),
        ));
        context.offline = true;
        context
    };
    let cache_artifact = |store: &Store, tx: &agentupdate::storage::model::UpdateTransaction| {
        let work = store.root.join("work").join(&tx.id);
        fs::create_dir_all(&work).unwrap();
        fs::copy(
            root.join("targets")
                .join(format!("{}.tgz", tx.installation.agent_id.key())),
            work.join("target.tgz"),
        )
        .unwrap();
    };
    let result = updater::queue::run(&mut store, &plan.id, false, |s, id| {
        let tx = s.transaction(id)?;
        cache_artifact(s, &tx);
        updater::execute_item(s, id, &contexts(&tx))
    })
    .unwrap();
    assert_eq!(result.state, "paused");
    assert_eq!(result.transactions[0].stage, Stage::Succeeded);
    assert_eq!(result.transactions[1].stage, Stage::PreparationFailed);
    let original = report
        .instances
        .iter()
        .find(|i| i.agent_id == AgentId::Gemini)
        .unwrap();
    let version = agentupdate::agents::discovery::verify(
        &DiscoveryContext {
            home: root.clone(),
            directories: vec![],
        },
        original.node_path.as_ref().unwrap(),
        &original.resolved_command,
        true,
    )
    .unwrap();
    assert_eq!(version.0, "0.62.0");
    let finished = updater::queue::run(&mut store, &plan.id, true, |_, _| {
        panic!("Failed items must not retry automatically")
    })
    .unwrap();
    assert_eq!(finished.transactions[0].stage, Stage::Succeeded);
    let target =
        serde_json::from_slice(&fs::read(root.join("targets/gemini.json")).unwrap()).unwrap();
    let next = store
        .create_plan(vec![PlannedItem {
            installation: original.clone(),
            target,
        }])
        .unwrap();
    // A fixture-only SQLite trigger requests cancellation exactly as installing is committed.
    let db = rusqlite::Connection::open(store.root.join("state.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER fixture_cancel_install AFTER UPDATE OF stage ON transactions WHEN NEW.stage='installing' BEGIN UPDATE plans SET cancel_requested=1 WHERE id=NEW.plan_id; END;").unwrap();
    drop(db);
    let cancelled = updater::queue::run(&mut store, &next.id, false, |s, id| {
        let tx = s.transaction(id)?;
        cache_artifact(s, &tx);
        updater::execute_item(s, id, &contexts(&tx))
    })
    .unwrap();
    assert_eq!(cancelled.state, "cancelled");
    assert_eq!(cancelled.transactions[0].stage, Stage::Succeeded);
    let verified = agentupdate::agents::discovery::verify(
        &DiscoveryContext {
            home: root.clone(),
            directories: vec![],
        },
        original.node_path.as_ref().unwrap(),
        &original.resolved_command,
        true,
    )
    .unwrap();
    assert_eq!(verified.0, "0.63.0");
    if let Some(destination) = std::env::var_os("AGENTUPDATE_UPDATE_EVIDENCE") {
        fs::write(destination,serde_json::to_vec_pretty(&serde_json::json!({"mixedBatchPaused":true,"codexSucceeded":true,"geminiPreparationFailedWithoutMutation":true,"successPreservedOnContinue":true,"installingCancellationWaitedForVerifiedSuccess":true,"geminiFinalVersion":verified.0})).unwrap()).unwrap();
    }
}

#[cfg(feature = "desktop-validation")]
#[test]
#[ignore = "Explicit isolated real-tool fault injection, never compiled into production releases"]
fn real_faults() {
    let root = PathBuf::from(std::env::var_os("AGENTUPDATE_DISCOVERY_FIXTURE").unwrap());
    let discovery = DiscoveryContext {
        home: root.clone(),
        directories: vec![root.join("codex/bin"), root.join("gemini/bin")],
    };
    let report = agentupdate::agents::discovery::discover(&discovery, &[]).unwrap();
    let mut store = Store::open(&root.join("app-data")).unwrap();
    let mut results = vec![];
    for installation in report.instances {
        let target: versions::AvailableVersion = serde_json::from_slice(
            &fs::read(
                root.join("targets")
                    .join(format!("{}.json", installation.agent_id.key())),
            )
            .unwrap(),
        )
        .unwrap();
        let mut context = npm::Context::current(&installation).unwrap();
        context.home = root.clone();
        context.cache = Some(PathBuf::from(
            std::env::var_os("AGENTUPDATE_NPM_CACHE").unwrap(),
        ));
        context.offline = true;
        let original =
            agentupdate::backup::snapshot::scan(installation.package_root.as_ref().unwrap(), None)
                .unwrap();
        for fault in [
            "AGENTUPDATE_TEST_FAIL_SWAP_AGENT",
            "AGENTUPDATE_TEST_FAIL_VERIFY_AGENT",
        ] {
            let plan = store
                .create_plan(vec![PlannedItem {
                    installation: installation.clone(),
                    target: target.clone(),
                }])
                .unwrap();
            let id = &plan.transactions[0].id;
            let work = store.root.join("work").join(id);
            fs::create_dir_all(&work).unwrap();
            fs::copy(
                root.join("targets")
                    .join(format!("{}.tgz", installation.agent_id.key())),
                work.join("target.tgz"),
            )
            .unwrap();
            std::env::set_var(fault, installation.agent_id.key());
            let result = updater::execute_item(&mut store, id, &context);
            std::env::remove_var(fault);
            assert!(result.is_err());
            assert_eq!(store.transaction(id).unwrap().stage, Stage::NeedsRecovery);
            if fault == "AGENTUPDATE_TEST_FAIL_SWAP_AGENT" {
                assert!(!installation.package_root.as_ref().unwrap().exists());
            }
            std::env::set_var(
                "AGENTUPDATE_TEST_FAIL_RESTORE_AGENT",
                installation.agent_id.key(),
            );
            assert!(updater::rollback::execute(&mut store, id, &context, false).is_err());
            std::env::remove_var("AGENTUPDATE_TEST_FAIL_RESTORE_AGENT");
            assert_eq!(store.transaction(id).unwrap().stage, Stage::RecoveryFailed);
            assert!(agentupdate::backup::load(&store, &store.transaction(id).unwrap()).is_ok());
            assert_eq!(
                updater::rollback::execute(&mut store, id, &context, false)
                    .unwrap()
                    .stage,
                Stage::Restored
            );
            assert_eq!(
                agentupdate::backup::snapshot::scan(
                    installation.package_root.as_ref().unwrap(),
                    None
                )
                .unwrap()
                .entries,
                original.entries
            );
            results.push(serde_json::json!({"agent":installation.agent_id,"fault":fault,"needsRecovery":true,"retryAfterRecoveryFailure":true,"originalPackageExact":true}));
        }
    }
    if let Some(path) = std::env::var_os("AGENTUPDATE_UPDATE_EVIDENCE") {
        fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
}
