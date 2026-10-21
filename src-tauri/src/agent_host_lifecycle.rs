//! Ownership and bounded shutdown for the desktop's own host process tree.
//! A borrowed endpoint never creates this guard. Unix children get a new PGID;
//! closing a window does not stop it. SIGKILL/crashes cannot run exit callbacks.

use std::{
    io,
    process::{Child, ChildStdout, Command},
    thread,
    time::{Duration, Instant},
};

const HOST_GRACE: Duration = Duration::from_secs(2);
const GROUP_GRACE: Duration = Duration::from_millis(250);
const POLL: Duration = Duration::from_millis(15);

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct ShutdownOutcome {
    pub forced: bool,
    pub host_reaped: bool,
}

pub(super) struct OwnedHostProcess {
    child: Child,
    #[cfg(unix)]
    group: libc::pid_t,
    host_grace: Duration,
    stopped: bool,
}

impl OwnedHostProcess {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command.spawn()?;
        #[cfg(unix)]
        let group = child.id() as libc::pid_t;
        Ok(Self {
            child,
            #[cfg(unix)]
            group,
            host_grace: HOST_GRACE,
            stopped: false,
        })
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    #[cfg(unix)]
    fn signal_group(&self, signal: libc::c_int) {
        // Defense in depth: never signal this desktop's own process group,
        // zero, or a group that was not minted from our child's PID.
        if self.group > 1
            && self.group == self.child.id() as libc::pid_t
            && self.group != unsafe { libc::getpgrp() }
        {
            unsafe { libc::kill(-self.group, signal) };
        }
    }

    #[cfg(unix)]
    fn group_alive(&self) -> bool {
        let result = unsafe { libc::kill(-self.group, 0) };
        result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    #[cfg(unix)]
    fn wait_until_quiet(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let host_reaped = matches!(self.child.try_wait(), Ok(Some(_)));
            if host_reaped && !self.group_alive() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(POLL);
        }
    }

    pub fn stop(&mut self) -> ShutdownOutcome {
        if self.stopped {
            return ShutdownOutcome {
                host_reaped: matches!(self.child.try_wait(), Ok(Some(_))),
                ..ShutdownOutcome::default()
            };
        }
        self.stopped = true;
        #[cfg(unix)]
        {
            // Give the host its normal JS lifecycle/DB close first. Do not
            // signal an already-reaped PID which the OS could reuse.
            if matches!(self.child.try_wait(), Ok(None)) {
                unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) };
            }
            if self.wait_until_quiet(self.host_grace) {
                return ShutdownOutcome {
                    forced: false,
                    host_reaped: true,
                };
            }
            self.signal_group(libc::SIGTERM);
            if self.wait_until_quiet(GROUP_GRACE) {
                return ShutdownOutcome {
                    forced: false,
                    host_reaped: true,
                };
            }
            self.signal_group(libc::SIGKILL);
            self.wait_until_quiet(GROUP_GRACE);
            ShutdownOutcome {
                forced: true,
                host_reaped: matches!(self.child.try_wait(), Ok(Some(_))),
            }
        }
        #[cfg(not(unix))]
        {
            // Keep the existing direct-child behavior on other platforms.
            // Whole-tree ownership there requires a platform job object.
            let _ = self.child.kill();
            let host_reaped = self.child.wait().is_ok();
            ShutdownOutcome {
                forced: true,
                host_reaped,
            }
        }
    }
}

impl Drop for OwnedHostProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(unix)]
mod exit_signals {
    use std::{
        io,
        sync::{
            atomic::{AtomicBool, AtomicI32, Ordering},
            Arc,
        },
        thread::{self, JoinHandle},
        time::Duration,
    };

    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static SIGNAL: AtomicI32 = AtomicI32::new(0);

    extern "C" fn receive_signal(signal: libc::c_int) {
        // Async-signal-safe: no allocation, locks, Tauri, logging, or child I/O.
        SIGNAL.store(signal, Ordering::Relaxed);
    }

    pub struct ExitSignalBridge {
        stop: Arc<AtomicBool>,
        watcher: Option<JoinHandle<()>>,
        previous: Vec<(libc::c_int, libc::sigaction)>,
    }

    impl ExitSignalBridge {
        pub fn install(on_signal: impl Fn(i32) + Send + 'static) -> io::Result<Self> {
            if INSTALLED
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "desktop exit signals already installed",
                ));
            }
            SIGNAL.store(0, Ordering::Relaxed);
            let mut previous = Vec::new();
            for signal in [libc::SIGTERM, libc::SIGINT] {
                let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
                let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
                action.sa_sigaction = receive_signal as *const () as libc::sighandler_t;
                action.sa_flags = libc::SA_RESTART;
                unsafe { libc::sigemptyset(&mut action.sa_mask) };
                if unsafe { libc::sigaction(signal, &action, &mut old) } != 0 {
                    let error = io::Error::last_os_error();
                    for (signal, old) in &previous {
                        unsafe { libc::sigaction(*signal, old, std::ptr::null_mut()) };
                    }
                    INSTALLED.store(false, Ordering::Release);
                    return Err(error);
                }
                previous.push((signal, old));
            }
            let stop = Arc::new(AtomicBool::new(false));
            let watcher_stop = stop.clone();
            let watcher = thread::Builder::new()
                .name("desktop-exit-signal".into())
                .spawn(move || {
                    while !watcher_stop.load(Ordering::Acquire) {
                        let signal = SIGNAL.swap(0, Ordering::Relaxed);
                        if signal != 0 {
                            on_signal(signal);
                            break;
                        }
                        thread::park_timeout(Duration::from_millis(40));
                    }
                });
            match watcher {
                Ok(watcher) => Ok(Self {
                    stop,
                    watcher: Some(watcher),
                    previous,
                }),
                Err(error) => {
                    for (signal, old) in previous {
                        unsafe { libc::sigaction(signal, &old, std::ptr::null_mut()) };
                    }
                    INSTALLED.store(false, Ordering::Release);
                    Err(error)
                }
            }
        }
    }

    impl Drop for ExitSignalBridge {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(watcher) = self.watcher.take() {
                watcher.thread().unpark();
                let _ = watcher.join();
            }
            for (signal, old) in &self.previous {
                unsafe { libc::sigaction(*signal, old, std::ptr::null_mut()) };
            }
            INSTALLED.store(false, Ordering::Release);
        }
    }
}

#[cfg(unix)]
pub(super) use exit_signals::ExitSignalBridge;

#[cfg(all(test, unix))]
#[path = "agent_host_lifecycle_tests.rs"]
mod tests;
