use super::{channel, forward, Event};
use std::{
    collections::VecDeque,
    process::{Command, Stdio},
    thread,
};

/// Drains both pipes concurrently and reports capture failure separately from
/// process status. The observer receives sanitized, bounded messages only.
pub(crate) fn run_command(
    mut command: Command,
    mut observe: impl FnMut(&str, &str),
) -> Result<(), String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| crate::log_display::line(&format!("无法启动：{error}")))?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("无法读取安装输出管道".into());
    };
    let (sender, receiver) = channel();
    let stdout_sender = sender.clone();
    let stdout_thread = match thread::Builder::new()
        .name("install-stdout".into())
        .spawn(move || forward(stdout, "stdout", stdout_sender))
    {
        Ok(handle) => handle,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("无法启动安装 stdout 读取线程".into());
        }
    };
    let stderr_thread = match thread::Builder::new()
        .name("install-stderr".into())
        .spawn(move || forward(stderr, "stderr", sender))
    {
        Ok(handle) => handle,
        Err(_) => {
            // Release backpressure before joining the first reader.
            drop(receiver);
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_thread.join();
            return Err("无法启动安装 stderr 读取线程".into());
        }
    };
    let mut tail = VecDeque::with_capacity(24);
    let mut capture_failed = false;
    for event in receiver {
        let (stream, message) = match event {
            Event::Line { stream, message } => (stream, message),
            Event::ReadFailed { stream, kind } => {
                capture_failed = true;
                ("error", format!("安装 {stream} 输出读取失败：{kind:?}"))
            }
        };
        observe(stream, &message);
        if tail.len() == 24 {
            tail.pop_front();
        }
        tail.push_back(message);
    }
    for joined in [stdout_thread.join(), stderr_thread.join()] {
        if joined.is_err() {
            capture_failed = true;
            observe("error", "安装输出读取线程异常结束");
        }
    }
    let status = child
        .wait()
        .map_err(|error| crate::log_display::line(&format!("等待安装进程失败：{error}")))?;
    completion(status.success(), capture_failed, &status.to_string(), tail)
}

fn completion(
    success: bool,
    capture_failed: bool,
    status: &str,
    tail: VecDeque<String>,
) -> Result<(), String> {
    if success && !capture_failed {
        return Ok(());
    }
    let reason = if capture_failed {
        "输出未完整读取，无法确认安装步骤正常完成"
    } else {
        "安装进程失败"
    };
    Err(crate::log_display::text(
        &format!(
            "{reason}（{status}）\n{}",
            tail.into_iter().collect::<Vec<_>>().join("\n")
        ),
        8000,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_logs_capture_failure_cannot_be_reported_as_success() {
        assert!(completion(true, false, "exit 0", VecDeque::new()).is_ok());
        for (success, capture_failed) in [(false, false), (true, true), (false, true)] {
            assert!(completion(success, capture_failed, "fixture", VecDeque::new()).is_err());
        }
    }

    #[test]
    fn installation_logs_spawn_failure_is_returned_without_raw_secrets() {
        let missing =
            std::env::temp_dir().join(format!("missing-install-{}", uuid::Uuid::new_v4()));
        assert!(!missing.exists());
        assert!(run_command(Command::new(missing), |_, _| panic!("no pipe was opened")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn installation_logs_local_process_drains_both_pipes_and_preserves_exit_failure() {
        for code in [0, 7] {
            let mut command = Command::new("/bin/sh");
            command.args([
                "-c",
                &format!(
                    "printf 'out\\rnext\\n'; printf 'password=private-value\\n' >&2; exit {code}"
                ),
            ]);
            let mut events = Vec::new();
            let result = run_command(command, |stream, text| {
                events.push((stream.to_string(), text.to_string()))
            });
            assert_eq!(result.is_ok(), code == 0);
            assert!(events.contains(&("stdout".into(), "out".into())));
            assert!(events.contains(&("stdout".into(), "next".into())));
            assert!(events.iter().any(|(stream, _)| stream == "stderr"));
            assert!(!format!("{events:?} {result:?}").contains("private-value"));
        }
    }
}
