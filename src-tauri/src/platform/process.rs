use crate::error::AppError;
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

// Backend-only process runner. Never registered as an IPC command.
pub fn run(
    executable: &Path,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    timeout: Duration,
    limit: usize,
) -> Result<String, AppError> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Let macOS use posix_spawn in the threaded desktop process, including Rosetta.
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|_| AppError::new("PROCESS_START", "无法启动已验证的本地工具。"))?;
    let child_pid = child.id();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let outcome = stdout
            .take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(outcome);
    });
    let start = Instant::now();
    let mut captured = None;
    loop {
        if captured.is_none() {
            if let Ok(result) = receiver.try_recv() {
                let bytes = match result {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        terminate(&mut child, child_pid);
                        return Err(AppError::new("PROCESS_OUTPUT", "无法读取工具输出。"));
                    }
                };
                if bytes.len() > limit {
                    terminate(&mut child, child_pid);
                    return Err(AppError::new(
                        "PROCESS_OUTPUT_LIMIT",
                        "工具输出超过允许范围。",
                    ));
                }
                captured = Some(bytes);
            }
        }
        match child.try_wait() {
            Ok(Some(status)) if captured.is_some() => {
                if !status.success() {
                    return Err(AppError::new(
                        "PROCESS_FAILED",
                        "工具检查未通过；未修改安装。",
                    ));
                }
                return String::from_utf8(captured.unwrap())
                    .map_err(|_| AppError::new("PROCESS_OUTPUT", "工具输出格式无效。"));
            }
            Err(_) => {
                terminate(&mut child, child_pid);
                return Err(AppError::new("PROCESS_WAIT", "无法确认工具进程状态。"));
            }
            _ => {}
        }
        if start.elapsed() >= timeout {
            terminate(&mut child, child_pid);
            return Err(AppError::new(
                "PROCESS_TIMEOUT",
                "工具检查超时；未修改安装。",
            ));
        }
        thread::sleep(Duration::from_millis(15));
    }
}

fn terminate(child: &mut std::process::Child, pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}
