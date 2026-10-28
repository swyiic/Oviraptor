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

