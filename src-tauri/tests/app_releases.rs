use agentupdate::releases::{self, Cache, FetchFailure, FetchResult};
#[test]
#[ignore = "Requires the release-check-fixture.json produced by package:mac"]
fn packaged_release_fixture_matches_both_architectures_and_build_version() {
    let path = std::env::var("AGENTUPDATE_RELEASE_FIXTURE").unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let latest = releases::parse(&payload).unwrap().unwrap();
    assert_eq!(latest.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(latest.tag, format!("v{}", env!("CARGO_PKG_VERSION")));
    let expected = format!("AgentUpdate-{}-macos-universal.dmg", latest.version);
    let mut fixture_cache = Cache::default();
    releases::apply(
        &mut fixture_cache,
        Ok(FetchResult::Published(Some(latest), None)),
        1000,
    );
    for architecture in ["aarch64", "x86_64"] {
        let result = releases::evaluate(&fixture_cache, "0.0.0", architecture, false);
        assert_eq!(result.state, "update_available");
        let asset = result.asset.unwrap();
        assert_eq!(asset.name, expected);
        assert_eq!(
            asset.url,
            format!(
                "{}/download/v{}/{}",
                releases::SOURCE,
                env!("CARGO_PKG_VERSION"),
                expected
            )
        );
        assert_eq!(
            releases::evaluate(
                &fixture_cache,
                env!("CARGO_PKG_VERSION"),
                architecture,
                false
            )
            .state,
            "up_to_date"
        );
    }
}
fn release(tag: &str, arches: &[&str]) -> serde_json::Value {
    let version = tag.trim_start_matches('v');
    serde_json::json!({"draft":false,"prerelease":false,"tag_name":tag,"html_url":format!("{}/tag/{tag}",releases::SOURCE),"published_at":"2026-10-12T00:00:00Z","body":"Release fixture notes","assets":arches.iter().map(|arch|{let name=format!("AgentUpdate-{version}-macos-{arch}.dmg");serde_json::json!({"name":name,"state":"uploaded","browser_download_url":format!("{}/download/{tag}/{name}",releases::SOURCE)})}).collect::<Vec<_>>()})
}
fn cache(tag: &str, arches: &[&str]) -> Cache {
    let mut c = Cache::default();
    releases::apply(
        &mut c,
        Ok(FetchResult::Published(
            releases::parse(&serde_json::json!([release(tag, arches)])).unwrap(),
            Some("etag-fixture".into()),
        )),
        1000,
    );
    c
}
#[test]
fn semantic_version_comparison_has_distinct_equal_newer_and_older_states() {
    let c = cache("v0.10.0", &["universal"]);
    assert_eq!(
        releases::evaluate(&c, "0.9.0", "aarch64", false).state,
        "update_available"
    );
    assert_eq!(
        releases::evaluate(&c, "0.10.0", "aarch64", false).state,
        "up_to_date"
    );
    assert_eq!(
        releases::evaluate(&c, "0.11.0", "aarch64", false).state,
        "local_newer"
    );
    let parsed = releases::parse(&serde_json::json!([
        release("v0.9.0", &[]),
        release("v0.10.0", &[])
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(parsed.version, "0.10.0");
}
#[test]
fn drafts_prereleases_invalid_tags_and_empty_releases_never_generate_stable_updates() {
    let mut draft = release("v99.0.0", &[]);
    draft["draft"] = true.into();
    let mut pre = release("v2.0.0-beta.1", &[]);
    pre["prerelease"] = true.into();
    let parsed = releases::parse(&serde_json::json!([
        draft,
        pre,
        release("vbad", &[]),
        release("0.11.0", &[]),
        release("v0.10.0", &[])
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(parsed.version, "0.10.0");
    let mut c = Cache::default();
    releases::apply(
        &mut c,
        Ok(FetchResult::Published(
            releases::parse(&serde_json::json!([])).unwrap(),
            None,
        )),
        1,
    );
    assert_eq!(
        releases::evaluate(&c, "0.1.0", "x86_64", false).state,
        "no_releases"
    );
    assert!(releases::parse(&serde_json::json!({"message":"rate limited"})).is_err());
}
#[test]
fn asset_selection_matches_architecture_and_rejects_external_or_unpublished_assets() {
    let c = cache("v0.10.0", &["aarch64", "x86_64", "universal"]);
    assert!(releases::evaluate(&c, "0.1.0", "aarch64", false)
        .asset
        .unwrap()
        .name
        .contains("aarch64"));
    assert!(releases::evaluate(&c, "0.1.0", "x86_64", false)
        .asset
        .unwrap()
        .name
        .contains("x86_64"));
    let c = cache("v0.10.0", &["x86_64"]);
    assert!(releases::evaluate(&c, "0.1.0", "aarch64", false)
        .asset
        .is_none());
    assert!(releases::evaluate(&c, "0.1.0", "unknown", false)
        .asset
        .is_none());
    let mut value = release("v0.10.0", &["universal"]);
    value["assets"][0]["browser_download_url"] = "https://example.com/wrong.dmg".into();
    assert!(releases::parse(&serde_json::json!([value]))
        .unwrap()
        .unwrap()
        .assets
        .is_empty());
    let mut value = release("v0.10.0", &[]);
    value["html_url"] = "https://example.com".into();
    assert!(releases::parse(&serde_json::json!([value])).is_err());
}
#[test]
fn daily_throttle_manual_checks_network_failure_cache_and_rate_backoff_are_distinct() {
    let mut c = cache("v0.10.0", &["universal"]);
    assert!(!releases::should_fetch(&c, false, 1001));
    assert!(releases::should_fetch(&c, true, 1001));
    assert!(releases::should_fetch(
        &c,
        false,
        1000 + 24 * 60 * 60 * 1000
    ));
    releases::apply(
        &mut c,
        Err(FetchFailure {
            code: "RELEASE_NETWORK",
            retry_after: 0,
        }),
        2000,
    );
    let result = releases::evaluate(&c, "0.1.0", "aarch64", true);
    assert_eq!(result.state, "failed");
    assert_eq!(result.latest.unwrap().version, "0.10.0");
    assert_eq!(result.successful_at, Some(1000));
    assert_eq!(result.checked_at, Some(2000));
    assert!(releases::should_fetch(&c, true, 2001));
    releases::apply(
        &mut c,
        Err(FetchFailure {
            code: "RELEASE_RATE_LIMIT",
            retry_after: 120_000,
        }),
        3000,
    );
    assert!(!releases::should_fetch(&c, true, 4000));
    assert!(releases::should_fetch(&c, true, 120_000));
    releases::apply(&mut c, Ok(FetchResult::NotModified), 130_000);
    assert_eq!(
        releases::evaluate(&c, "0.1.0", "aarch64", false).state,
        "update_available"
    );
    assert!(c.error_code.is_none());
}
#[test]
#[ignore = "Live fixed GitHub endpoint; output records actual state rather than asserting a release exists"]
fn live_release_check() {
    let root = std::env::temp_dir().join(format!("agentupdate-release-{}", uuid::Uuid::new_v4()));
    let store = agentupdate::storage::Store::open(&root).unwrap();
    let result = releases::check(&store.root, true, "0.1.0", std::env::consts::ARCH).unwrap();
    if let Some(dest) = std::env::var_os("AGENTUPDATE_RELEASE_EVIDENCE") {
        std::fs::write(dest, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn persisted_cache_reevaluates_current_build_and_corrupt_links_are_preserved_without_opening() {
    let root = std::env::temp_dir().join(format!(
        "agentupdate-release-cache-{}",
        uuid::Uuid::new_v4()
    ));
    let store = agentupdate::storage::Store::open(&root).unwrap();
    let mut c = cache("v0.10.0", &["universal"]);
    c.checked_at = Some(agentupdate::storage::now());
    c.successful_at = c.checked_at;
    let path = store.root.join("release-cache.json");
    agentupdate::platform::files::write_atomic(&path, &serde_json::to_vec(&c).unwrap()).unwrap();
    let result = releases::check(&store.root, false, "0.10.0", "aarch64").unwrap();
    assert!(result.cached);
    assert_eq!(result.state, "up_to_date");
    let result = releases::check(&store.root, false, "0.11.0", "x86_64").unwrap();
    assert_eq!(result.state, "local_newer");
    c.latest.as_mut().unwrap().url = "file:///etc/hosts".into();
    let bad = serde_json::to_vec(&c).unwrap();
    std::fs::write(&path, &bad).unwrap();
    assert!(releases::check(&store.root, false, "0.1.0", "aarch64").is_err());
    assert!(releases::open_link(&store.root, false, "0.1.0", "aarch64").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bad);
    std::fs::remove_dir_all(root).unwrap();
}
