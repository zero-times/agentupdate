use agentupdate::{
    agents::{discovery::Installation, AgentId},
    cli_launch,
};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("agentupdate-launch-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(fs::canonicalize(root).unwrap())
    }
    fn directory(&self, name: &str) -> PathBuf {
        let p = self.0.join(name);
        fs::create_dir(&p).unwrap();
        p
    }
    fn installation(&self) -> Installation {
        let bin = self.directory("bin ' \" $(touch unsafe) `touch unsafe`");
        let command = bin.join("codex");
        fs::write(&command, "#!/bin/sh\nprintf '%s' \"$PWD\" > launched-cwd\n").unwrap();
        fs::set_permissions(&command, fs::Permissions::from_mode(0o700)).unwrap();
        Installation {
            id: "fixture".into(),
            agent_id: AgentId::Codex,
            command_path: command.clone(),
            resolved_command: command,
            package_root: None,
            prefix: None,
            node_path: None,
            version: None,
            architecture: "aarch64".into(),
            source: "fixture".into(),
            supported: false,
            version_verified: false,
            reason: None,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn recent_projects_persist_edit_deduplicate_and_reject_invalid_directories() {
    let f = Fixture::new();
    let data = f.0.join("data");
    assert!(cli_launch::projects(&data).unwrap().is_empty());
    let first = f.directory("first");
    let second = f.directory("second");
    let first = first.to_str().unwrap();
    let second = second.to_str().unwrap();
    cli_launch::save_project(&data, first, None).unwrap();
    cli_launch::save_project(&data, second, None).unwrap();
    assert_eq!(cli_launch::projects(&data).unwrap(), vec![second, first]);
    cli_launch::save_project(&data, first, Some(second)).unwrap();
    assert_eq!(cli_launch::projects(&data).unwrap(), vec![first]);
    for invalid in ["relative", "/does-not-exist-age24", "/tmp/\ninvalid"] {
        assert!(cli_launch::save_project(&data, invalid, None).is_err());
    }
    let file = f.0.join("file");
    fs::write(&file, "").unwrap();
    assert!(cli_launch::save_project(&data, file.to_str().unwrap(), None).is_err());
    assert_eq!(cli_launch::projects(&data).unwrap(), vec![first]);
    assert_eq!(
        fs::metadata(data.join("cli-projects.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    for n in 0..22 {
        let p = f.directory(&format!("project-{n}"));
        cli_launch::save_project(&data, p.to_str().unwrap(), None).unwrap();
    }
    let paths = cli_launch::projects(&data).unwrap();
    assert_eq!(paths.len(), 20);
    assert!(paths[0].ends_with("project-21"));
}

#[test]
fn shell_launch_preserves_hostile_paths_and_selected_instance() {
    let f = Fixture::new();
    let installation = f.installation();
    let project = f.directory("中文 project ' \" ; $(touch unsafe) `touch unsafe`");
    let project = cli_launch::project_directory(project.to_str().unwrap()).unwrap();
    let command = cli_launch::terminal_command(&installation, &project).unwrap();
    for shell in ["/bin/sh", "/bin/zsh"] {
        assert!(Command::new(shell)
            .args(["-c", &command])
            .current_dir(&f.0)
            .status()
            .unwrap()
            .success());
        assert_eq!(
            fs::read_to_string(project.join("launched-cwd")).unwrap(),
            project.to_str().unwrap()
        );
        assert!(!f.0.join("unsafe").exists());
        assert!(!project.join("unsafe").exists());
    }
    fs::remove_file(&installation.command_path).unwrap();
    assert!(cli_launch::terminal_command(&installation, &project).is_err());
}

#[test]
fn deepseek_launch_requires_profile_and_defaults_to_web() {
    let f = Fixture::new();
    let mut installation = f.installation();
    installation.agent_id = AgentId::Deepseek;
    fs::write(&installation.command_path, "#!/bin/sh\n[ \"$1\" = '--profile' ] && [ -n \"$2\" ] || exit 2\nprintf '%s' \"$2\" > launched-profile\n").unwrap();
    let project = f.directory("project");
    for profile in [None, Some("my-tui")] {
        let command =
            cli_launch::terminal_command_with_profile(&installation, &project, profile).unwrap();
        assert!(Command::new("/bin/zsh")
            .args(["-c", &command])
            .status()
            .unwrap()
            .success());
        assert_eq!(
            fs::read_to_string(project.join("launched-profile")).unwrap(),
            profile.unwrap_or("web")
        );
    }
    for profile in [
        "",
        "desktop",
        "DESKTOP",
        "..",
        "../web",
        "web;touch unsafe",
        "$(touch unsafe)",
        "--help",
    ] {
        assert!(
            cli_launch::terminal_command_with_profile(&installation, &project, Some(profile))
                .is_err()
        );
    }
    installation.agent_id = AgentId::Codex;
    assert!(
        cli_launch::terminal_command_with_profile(&installation, &project, Some("web")).is_err()
    );
}

// Opt-in native Terminal integration; starts only a fixture, never a real Agent.
#[test]
#[ignore = "opens macOS Terminal; run explicitly for native launch acceptance"]
fn native_terminal_runs_fixture_in_selected_project() {
    let f = Fixture::new();
    let installation = f.installation();
    let project = f.directory("中文 project ' \" ; $(touch unsafe) `touch unsafe`");
    let project = cli_launch::launch(&installation, project.to_str().unwrap()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !project.join("launched-cwd").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(
        fs::read_to_string(project.join("launched-cwd")).unwrap(),
        project.to_str().unwrap()
    );
    assert!(!project.join("unsafe").exists());
}
