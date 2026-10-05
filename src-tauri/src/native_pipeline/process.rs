//! §10.2 item 7 — one bounded way to run an external program. Arguments are always an
//! array (never a shell string), output is capped, and both a deadline and a cancel
//! check are mandatory, so no caller can invent a fourth variant of this.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};

#[path = "process_log/mod.rs"]
pub(crate) mod log;
#[path = "process_output.rs"]
pub(crate) mod output;
use output::pump;

pub(crate) struct ObservedRun {
    pub run: BoundedRun,
    pub log_error: Option<String>,
}

type Observer<'a> = dyn FnMut(&log::Event) -> Result<(), String> + 'a;

pub(crate) fn run_observed(
    program: &Path,
    args: &[String],
    cwd: Option<&Path>,
    limits: &ProcessLimits,
    cancelled: &dyn Fn() -> bool,
    observe: &mut dyn FnMut(&log::Event) -> Result<(), String>,
) -> Result<ObservedRun, ProcessError> {
    run_inner(program, args, cwd, limits, cancelled, Some(observe))
}

#[derive(Clone, Debug)]
pub struct ProcessLimits {
    pub timeout: Duration,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub poll: Duration,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(600),
            stdout_bytes: 4 * 1024 * 1024,
            stderr_bytes: 1024 * 1024,
            poll: Duration::from_millis(20),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BoundedRun {
    pub exit: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    pub duration: Duration,
    /// An incomplete drain/cleanup must never be reported as successful execution.
    pub cleanup_error: Option<String>,
}

impl BoundedRun {
    pub fn succeeded(&self) -> bool {
        !self.timed_out && !self.cancelled && self.cleanup_error.is_none() && self.exit == Some(0)
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }
}

/// A missing program is reported apart from a failed one, because the caller has to
/// record a coverage gap rather than a scan failure (§10.2 item 9).
#[derive(Clone, Debug)]
pub enum ProcessError {
    Missing { program: String, reason: String },
    Launch { program: String, reason: String },
}

impl ProcessError {
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Missing { program, reason } => format!("缺少可执行文件 {program}：{reason}"),
            Self::Launch { program, reason } => format!("无法启动 {program}：{reason}"),
        }
    }
}

#[cfg(unix)]
pub(crate) fn interruptible(pipe: &impl std::os::fd::AsRawFd) -> std::io::Result<()> {
    let fd = pipe.as_raw_fd();
    // SAFETY: fd is borrowed from a live, exclusively owned child pipe. Neither
    // call consumes it; flags are changed only for this owned read endpoint.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn interruptible<T>(_pipe: &T) -> std::io::Result<()> {
    Ok(())
}

pub(crate) struct OwnedChild(pub(crate) Child, bool);

impl OwnedChild {
    pub(crate) fn new(child: Child) -> Self {
        Self(child, false)
    }
    pub(crate) fn stop(&mut self) -> Option<String> {
        if self.1 {
            return None;
        }
        self.1 = true;
        let mut error = None;
        #[cfg(unix)]
        {
            // SAFETY: process_group(0) created a new group with our child's PID.
            // Only that group is targeted, never the parent/application group.
            let result = unsafe { libc::kill(-(self.0.id() as i32), libc::SIGKILL) };
            if result != 0 {
                let failure = std::io::Error::last_os_error();
                if failure.raw_os_error() != Some(libc::ESRCH) {
                    error = Some(format!("process_group_cleanup:{failure}"));
                }
            }
        }
        let _ = self.0.kill();
        // Never block indefinitely on a child which the OS could not terminate.
        let started = Instant::now();
        loop {
            match self.0.try_wait() {
                Ok(Some(_)) => break,
                Err(failure) => {
                    error = Some(format!("process_reap:{failure}"));
                    break;
                }
                Ok(None) if started.elapsed() >= Duration::from_secs(1) => {
                    error = Some("process_cleanup_deadline".into());
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        error
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        // The caller disarms the guard after explicit cleanup. This handles early
        // errors (including configuring pipes) and unwind while the child is alive.
        let _ = self.stop();
    }
}

pub fn run(
    program: &Path,
    args: &[String],
    cwd: Option<&Path>,
    limits: &ProcessLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<BoundedRun, ProcessError> {
    run_inner(program, args, cwd, limits, cancelled, None).map(|observed| observed.run)
}

fn run_inner(
    program: &Path,
    args: &[String],
    cwd: Option<&Path>,
    limits: &ProcessLimits,
    cancelled: &dyn Fn() -> bool,
    mut observe: Option<&mut Observer<'_>>,
) -> Result<ObservedRun, ProcessError> {
    let channel = observe.as_ref().map(|_| output::Channel::new());
    let mut log_error = None;
    let started = Instant::now();
    let already_cancelled = cancelled();
    if already_cancelled || limits.timeout.is_zero() {
        return Ok(ObservedRun {
            log_error: None,
            run: BoundedRun {
                exit: None,
                stdout: vec![],
                stderr: vec![],
                truncated: false,
                timed_out: !already_cancelled,
                cancelled: already_cancelled,
                duration: started.elapsed(),
                cleanup_error: None,
            },
        });
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd.unwrap_or_else(|| Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = OwnedChild(
        command.spawn().map_err(|error| {
            let text = error.to_string();
            let name = program.display().to_string();
            if error.kind() == std::io::ErrorKind::NotFound {
                ProcessError::Missing {
                    program: name,
                    reason: text,
                }
            } else {
                ProcessError::Launch {
                    program: name,
                    reason: text,
                }
            }
        })?,
        false,
    );
    for result in [
        child.0.stdout.as_ref().map(interruptible),
        child.0.stderr.as_ref().map(interruptible),
    ]
    .into_iter()
    .flatten()
    {
        result.map_err(|error| ProcessError::Launch {
            program: program.display().to_string(),
            reason: format!("pipe_setup:{error}"),
        })?;
    }
    let (out_sender, out_receiver) = mpsc::channel();
    let (err_sender, err_receiver) = mpsc::channel();
    // The reader threads only ever need the two caps, so they take copies rather than
    // borrowing this call's limits.
    let stdout_cap = limits.stdout_bytes;
    let stderr_cap = limits.stderr_bytes;
    let stop_readers = Arc::new(AtomicBool::new(false));
    if let Some(pipe) = child.0.stdout.take() {
        let stop = Arc::clone(&stop_readers);
        let observer = channel.as_ref().map(|channel| channel.pipe("stdout"));
        std::thread::spawn(move || pump(pipe, stdout_cap, stop, out_sender, observer));
    } else {
        let _ = out_sender.send((Vec::new(), false));
    }
    if let Some(pipe) = child.0.stderr.take() {
        let stop = Arc::clone(&stop_readers);
        let observer = channel.as_ref().map(|channel| channel.pipe("stderr"));
        std::thread::spawn(move || pump(pipe, stderr_cap, stop, err_sender, observer));
    } else {
        let _ = err_sender.send((Vec::new(), false));
    }
    let mut timed_out = false;
    let mut was_cancelled = false;
    let mut status = None;
    loop {
        // Cancellation wins before observing a quick success or spawning a next step.
        if cancelled() {
            was_cancelled = true;
            break;
        }
        if started.elapsed() >= limits.timeout {
            timed_out = true;
            break;
        }
        if let (Some(channel), Some(observer)) = (&channel, observe.as_deref_mut()) {
            channel.deliver(observer, &mut log_error, false);
        }
        match child.0.try_wait() {
            Ok(Some(exit)) => {
                status = Some(exit);
                break;
            }
            Ok(None) => {}
            Err(error) => {
                stop_readers.store(true, Ordering::Release);
                return Err(ProcessError::Launch {
                    program: program.display().to_string(),
                    reason: error.to_string(),
                });
            }
        }
        std::thread::sleep(
            limits
                .poll
                .clamp(Duration::from_millis(1), Duration::from_millis(20))
                .min(limits.timeout.saturating_sub(started.elapsed())),
        );
    }
    // Even normal exit must not leave descendants running with inherited pipes.
    let mut cleanup_error = child.stop();
    stop_readers.store(true, Ordering::Release);
    let mut collect = |receiver: mpsc::Receiver<(Vec<u8>, bool)>| {
        receiver
            .recv_timeout(Duration::from_millis(250))
            .unwrap_or_else(|_| {
                cleanup_error = Some("process_output_drain_incomplete".into());
                (vec![], true)
            })
    };
    let (stdout, out_cut) = collect(out_receiver);
    let (stderr, err_cut) = collect(err_receiver);
    if let (Some(channel), Some(observer)) = (&channel, observe) {
        if cleanup_error.is_some() {
            log_error.get_or_insert_with(|| "native_process_log_drain_incomplete".into());
        }
        channel.finish(observer, &mut log_error);
    }
    Ok(ObservedRun {
        log_error,
        run: BoundedRun {
            exit: status.and_then(|exit| exit.code()),
            stdout,
            stderr,
            truncated: out_cut || err_cut,
            timed_out,
            cancelled: was_cancelled,
            duration: started.elapsed(),
            cleanup_error,
        },
    })
}

#[cfg(all(test, unix))]
#[path = "process_observer_tests.rs"]
mod observer_tests;
