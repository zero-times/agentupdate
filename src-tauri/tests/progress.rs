use agentupdate::{
    agents::versions::update_state,
    error::AppError,
    progress::{self, Phase, Progress, Reporter},
};
use std::{
    io::{self, Cursor, Read},
    sync::{Arc, Mutex},
};
fn recorder() -> (Arc<Mutex<Vec<Progress>>>, Reporter) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let log = events.clone();
    (events, Arc::new(move |p| log.lock().unwrap().push(p)))
}
#[test]
fn byte_progress_is_measured_and_short_or_excessive_streams_cannot_complete() {
    let (events, reporter) = recorder();
    progress::with_reporter(Some(reporter), || {
        for (target, total) in [("known", Some(200_000)), ("unknown", None)] {
            let mut bytes = Vec::new();
            progress::operation(target, || {
                assert_eq!(
                    progress::copy_download(
                        Cursor::new(vec![7; 200_000]),
                        &mut bytes,
                        300_000,
                        total
                    )
                    .unwrap(),
                    200_000
                );
                progress::phase(Phase::Verifying);
                Ok(())
            })
            .unwrap();
            assert_eq!(bytes, vec![7; 200_000]);
        }
        for (target, length, total, limit) in
            [("short", 10, Some(11), 20), ("too_big", 21, None, 20)]
        {
            assert!(progress::operation(target, || {
                progress::copy_download(Cursor::new(vec![0; length]), Vec::new(), limit, total)
                    .map_err(|_| AppError::new("DOWNLOAD_INTERRUPTED", "test"))?;
                Ok(())
            })
            .is_err());
        }
    });
    let events = events.lock().unwrap();
    for target in ["known", "unknown"] {
        let e: Vec<_> = events.iter().filter(|p| p.target == target).collect();
        assert_eq!(e[0].phase, Phase::Checking);
        assert!(e.iter().any(|p| p.phase == Phase::Downloading
            && p.downloaded_bytes == 200_000
            && p.total_bytes == Some(200_000)));
        assert_eq!(e.last().unwrap().phase, Phase::Succeeded);
    }
    for target in ["short", "too_big"] {
        let e: Vec<_> = events.iter().filter(|p| p.target == target).collect();
        assert!(!e.iter().any(|p| p.phase == Phase::Succeeded));
        assert_eq!(e.last().unwrap().phase, Phase::Failed);
    }
    let unknown = events
        .iter()
        .find(|p| p.target == "unknown" && p.phase == Phase::Downloading)
        .unwrap();
    assert_eq!(unknown.total_bytes, None);
}
struct Broken;
impl Read for Broken {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("interrupted"))
    }
}
#[test]
fn cancellation_restart_and_worker_scopes_never_report_false_completion() {
    let (events, reporter) = recorder();
    progress::with_reporter(Some(reporter), || {
        progress::operation("cancel", || {
            progress::phase(Phase::Cancelled);
            Ok(())
        })
        .unwrap();
        progress::operation("self", || {
            progress::phase(Phase::Restarting);
            Ok(())
        })
        .unwrap();
        assert!(progress::operation("interrupted", || {
            progress::copy_download(Broken, Vec::new(), 10, Some(10))
                .map_err(|_| AppError::new("DOWNLOAD_INTERRUPTED", "test"))
        })
        .is_err());
    });
    progress::phase(Phase::Succeeded); // Reused worker must not reuse a prior command's channel.
    let events = events.lock().unwrap();
    assert!(!events.iter().any(|p| p.phase == Phase::Succeeded));
    assert_eq!(
        events
            .iter()
            .filter(|p| p.target == "cancel")
            .last()
            .unwrap()
            .phase,
        Phase::Cancelled
    );
    assert_eq!(
        events
            .iter()
            .filter(|p| p.target == "self")
            .last()
            .unwrap()
            .phase,
        Phase::Restarting
    );
    assert_eq!(events.last().unwrap().phase, Phase::Failed);
}
#[test]
fn semantic_version_comparison_hides_upgrades_for_equal_newer_unknown_and_metadata_only_changes() {
    for (local, latest, expected) in [
        (Some("0.9.0"), "0.10.0", "update_available"),
        (Some("1.0.0-rc.9"), "1.0.0-rc.10", "update_available"),
        (Some("1.0.0-rc.10"), "1.0.0", "update_available"),
        (Some("1.0.0"), "1.0.0-rc.10", "local_newer"),
        (Some("1.0.0"), "1.0.0", "up_to_date"),
        (Some("1.0.0+local"), "1.0.0+upstream", "up_to_date"),
        (Some("1.1.0"), "1.0.0", "local_newer"),
        (Some("unknown"), "1.0.0", "unknown_version"),
        (None, "1.0.0", "unknown_version"),
    ] {
        assert_eq!(update_state(local, latest), expected);
    }
}
