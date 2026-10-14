use agentupdate::{agents::AgentId, service};

#[test]
fn unknown_or_path_agent_identifiers_are_rejected() {
    for input in [
        "unknown-agent",
        "/usr/bin/env",
        "../codex",
        "codex; echo unsafe",
        "",
    ] {
        assert!(serde_json::from_value::<AgentId>(serde_json::json!(input)).is_err());
    }
    assert!(serde_json::from_value::<AgentId>(serde_json::json!("codex")).is_ok());
    assert!(serde_json::from_value::<AgentId>(serde_json::json!("claude")).is_ok());
}

#[test]
fn scaffold_never_reports_discovery_or_updates_as_ready() {
    let overview = service::overview().unwrap();
    assert_eq!(overview.agents.len(), 4);
    assert!(overview
        .agents
        .iter()
        .all(|a| a.detection_state == "not_scanned" && !a.update_enabled));
    assert!(overview.backup.required_before_update);
    assert_eq!(overview.backup.state, "available");
    assert_eq!(overview.storage.state, "available");
    assert_eq!(overview.updater.state, "available");
    assert_eq!(overview.app_release.state, "not_checked");
}
