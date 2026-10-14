#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    platform: &'static str,
    architecture: &'static str,
    os_version: String,
    child_process: bool,
    isolated_file_roundtrip: bool,
    restrictive_file_permissions: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Checks {
    restricted_command_denied: bool,
    arbitrary_command_denied: bool,
    arbitrary_file_denied: bool,
}

fn platform_probe() -> Result<Report, String> {
    #[cfg(not(target_os = "macos"))]
    return Err("This experiment only validates macOS".into());

    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // No command, arguments or path is supplied by the frontend.
        let mut child = Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let start = Instant::now();
        loop {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                break;
            }
            if start.elapsed() > Duration::from_secs(3) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Fixed subprocess timed out".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("Fixed subprocess failed".into());
        }
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("agentupdate-spike-{}-{id}", std::process::id()));
        fs::create_dir(&root).map_err(|e| e.to_string())?;
        let result = (|| -> io::Result<(bool, bool)> {
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
            let file = root.join("probe.txt");
            let mut handle = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&file)?;
            use std::io::Write;
            handle.write_all(b"agentupdate-isolated-fixture")?;
            handle.sync_all()?;
            let bytes_match = fs::read(&file)? == b"agentupdate-isolated-fixture";
            let modes_match = fs::metadata(&file)?.permissions().mode() & 0o777 == 0o600
                && fs::metadata(&root)?.permissions().mode() & 0o777 == 0o700;
            Ok((bytes_match, modes_match))
        })();
        fs::remove_dir_all(&root).map_err(|e| e.to_string())?;
        let (isolated_file_roundtrip, restrictive_file_permissions) =
            result.map_err(|e| e.to_string())?;
        Ok(Report {
            platform: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            os_version: String::from_utf8(output.stdout)
                .map_err(|e| e.to_string())?
                .trim()
                .into(),
            child_process: true,
            isolated_file_roundtrip,
            restrictive_file_permissions,
        })
    }
}

#[tauri::command]
async fn run_validation() -> Result<Report, String> {
    tauri::async_runtime::spawn_blocking(platform_probe)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn admin_only_probe() -> &'static str {
    "This command must be denied by the capability manifest"
}

#[tauri::command]
fn finish_validation(app: tauri::AppHandle, checks: Checks) -> Result<(), String> {
    if let Ok(file) = std::env::var("AGENTUPDATE_SPIKE_EVIDENCE_FILE") {
        // The destination is set by the developer's test runner, never by the WebView.
        let evidence = serde_json::json!({ "report": platform_probe()?, "webviewChecks": checks });
        fs::write(
            file,
            serde_json::to_vec_pretty(&evidence).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    if std::env::var("AGENTUPDATE_SPIKE_AUTOTEST").as_deref() == Ok("1") {
        app.exit(0);
    }
    Ok(())
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--smoke") {
        match platform_probe() {
            Ok(report) => println!("{}", serde_json::to_string_pretty(&report).unwrap()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            run_validation,
            finish_validation,
            admin_only_probe
        ])
        .run(tauri::generate_context!())
        .expect("Could not run desktop experiment");
}

#[cfg(test)]
mod tests {
    #[test]
    fn platform_process_and_private_file_roundtrip() {
        let report = super::platform_probe().unwrap();
        assert!(
            report.child_process
                && report.isolated_file_roundtrip
                && report.restrictive_file_permissions
        );
    }
}
