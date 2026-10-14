#![allow(dead_code)]
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
    updater::npm,
};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::PathBuf,
};
pub struct Fixture {
    pub root: PathBuf,
    pub store: Store,
    pub id: String,
    pub context: npm::Context,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
pub fn fixture(config_present: bool) -> Fixture {
    fixture_at(config_present, Stage::Succeeded)
}
pub fn fixture_at(config_present: bool, final_stage: Stage) -> Fixture {
    let root = std::env::temp_dir().join(format!("agentupdate-rollback-{}", uuid::Uuid::new_v4()));
    let package = root.join("prefix/lib/node_modules/@openai/codex");
    fs::create_dir_all(&package).unwrap();
    let bin = root.join("prefix/bin");
    fs::create_dir(&bin).unwrap();
    fs::write(
        package.join("package.json"),
        br#"{"name":"@openai/codex","version":"1.0.0","bin":{"codex":"cli.js"}}"#,
    )
    .unwrap();
    fs::write(package.join("cli.js"),"#!/usr/bin/env node\nconsole.log(process.argv.includes('--help')?'fixture help':'1.0.0');\n").unwrap();
    fs::set_permissions(package.join("cli.js"), fs::Permissions::from_mode(0o700)).unwrap();
    symlink(package.join("cli.js"), bin.join("codex")).unwrap();
    symlink("/opt/homebrew/bin/node", bin.join("node")).unwrap();
    if config_present {
        fs::create_dir(root.join(".codex")).unwrap();
        fs::set_permissions(root.join(".codex"), fs::Permissions::from_mode(0o750)).unwrap();
        fs::write(root.join(".codex/config.toml"), b"original fixture").unwrap();
        fs::set_permissions(
            root.join(".codex/config.toml"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
    }
    let installation = discover(
        &DiscoveryContext {
            home: root.clone(),
            directories: vec![bin],
        },
        &[],
    )
    .unwrap()
    .instances
    .into_iter()
    .find(|i| i.agent_id == AgentId::Codex)
    .unwrap();
    assert!(installation.supported);
    let mut store = Store::open(&root.join("app-data")).unwrap();
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
    let id = plan.transactions[0].id.clone();
    store
        .transition(&id, Stage::Queued, Stage::Preflight, None)
        .unwrap();
    let tx = store
        .transition(&id, Stage::Preflight, Stage::BackingUp, None)
        .unwrap();
    backup::prepare(&mut store, &tx, &root).unwrap();
    if final_stage != Stage::Prepared {
        store
            .transition(&id, Stage::Prepared, Stage::Installing, None)
            .unwrap();
        fs::write(package.join("cli.js"),"#!/usr/bin/env node\nconsole.log(process.argv.includes('--help')?'fixture help':'1.1.0');\n").unwrap();
    }
    if final_stage != Stage::Installing && final_stage != Stage::Prepared {
        store
            .transition(&id, Stage::Installing, Stage::Verifying, None)
            .unwrap();
        if final_stage != Stage::Verifying {
            if final_stage == Stage::Recovering {
                store
                    .transition(&id, Stage::Verifying, Stage::Succeeded, None)
                    .unwrap();
                store
                    .transition(&id, Stage::Succeeded, Stage::Recovering, None)
                    .unwrap();
            } else {
                store
                    .transition(&id, Stage::Verifying, final_stage, None)
                    .unwrap();
            }
        }
    }
    let context = npm::Context {
        home: root.clone(),
        npm: PathBuf::new(),
        cache: None,
        offline: true,
    };
    Fixture {
        root,
        store,
        id,
        context,
    }
}
