//! Per-command progress channels. No paths, URLs, credentials or process output cross IPC.
use crate::error::AppError;
use serde::Serialize;
use std::{
    cell::RefCell,
    io::{self, Read, Write},
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Checking,
    BackingUp,
    Downloading,
    Staging,
    Installing,
    Verifying,
    Restarting,
    Succeeded,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub target: String,
    pub phase: Phase,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error_code: Option<String>,
}
pub type Reporter = Arc<dyn Fn(Progress) + Send + Sync>;
#[derive(Clone)]
struct Context {
    reporter: Option<Reporter>,
    current: Progress,
}
thread_local! {
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}
struct Scope(Option<Context>);
impl Drop for Scope {
    fn drop(&mut self) {
        CONTEXT.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}
pub fn with_reporter<T>(reporter: Option<Reporter>, job: impl FnOnce() -> T) -> T {
    let _scope = Scope(CONTEXT.with(|slot| {
        slot.replace(Some(Context {
            reporter,
            current: Progress {
                target: String::new(),
                phase: Phase::Checking,
                downloaded_bytes: 0,
                total_bytes: None,
                error_code: None,
            },
        }))
    }));
    job()
}
pub fn with_channel<T>(
    channel: Option<tauri::ipc::Channel<Progress>>,
    job: impl FnOnce() -> T,
) -> T {
    let reporter = channel.map(|channel| {
        Arc::new(move |p| {
            let _ = channel.send(p);
        }) as Reporter
    });
    with_reporter(reporter, job)
}
fn report(update: impl FnOnce(&mut Progress)) {
    let outgoing = CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let context = slot.as_mut()?;
        update(&mut context.current);
        Some((context.reporter.clone()?, context.current.clone()))
    });
    if let Some((send, progress)) = outgoing {
        send(progress);
    }
}
pub fn phase(phase: Phase) {
    report(|p| {
        p.phase = phase;
        p.downloaded_bytes = 0;
        p.total_bytes = None;
        p.error_code = None;
    });
    #[cfg(feature = "desktop-validation")]
    if let Some(ms) = std::env::var("AGENTUPDATE_TEST_PROGRESS_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| *ms <= 250)
    {
        std::thread::sleep(Duration::from_millis(ms));
    }
}
pub fn downloaded(bytes: u64, total: Option<u64>) {
    report(|p| {
        p.phase = Phase::Downloading;
        p.downloaded_bytes = bytes;
        p.total_bytes = total.filter(|n| *n > 0);
        p.error_code = None;
    });
}
pub fn operation<T>(
    target: &str,
    job: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    report(|p| {
        p.target = target.into();
        p.phase = Phase::Checking;
        p.downloaded_bytes = 0;
        p.total_bytes = None;
        p.error_code = None;
    });
    let result = job();
    match &result {
        Ok(_) => {
            let terminal = CONTEXT.with(|slot| {
                slot.borrow().as_ref().is_some_and(|c| {
                    matches!(c.current.phase, Phase::Restarting | Phase::Cancelled)
                })
            });
            if !terminal {
                phase(Phase::Succeeded);
            }
        }
        Err(e) => report(|p| {
            p.phase = if e.code == "CANCELLED" {
                Phase::Cancelled
            } else {
                Phase::Failed
            };
            p.error_code = Some(e.code.into());
        }),
    }
    result
}
/// Bounded streaming with measured byte progress. An unknown length stays indeterminate.
pub fn copy_download(
    mut reader: impl Read,
    mut writer: impl Write,
    limit: u64,
    total: Option<u64>,
) -> io::Result<u64> {
    downloaded(0, total);
    let mut size = 0;
    let mut last = Instant::now();
    let mut buffer = [0; 65536];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > limit {
            return Err(io::Error::other("download size limit"));
        }
        writer.write_all(&buffer[..n])?;
        if last.elapsed() >= Duration::from_millis(150) {
            downloaded(size, total);
            last = Instant::now();
        }
    }
    if total.is_some_and(|n| n != size) {
        return Err(io::Error::other("incomplete download"));
    }
    downloaded(size, Some(size));
    Ok(size)
}
