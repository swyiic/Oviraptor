use super::*;
use std::os::unix::fs::PermissionsExt;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ovi-worker-probe-test-{}", Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn script(&self, name: &str, body: &str) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path.to_string_lossy().into_owned()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn disabled_and_idle_worker_status_is_a_pure_snapshot() {
    let control = WorkerServiceControl::default();
    for _ in 0..100 {
        let value = local_worker_snapshot(&control, false, 19427, "synthetic-token".into());
        assert!(!value.enabled);
        assert!(!value.running);
        assert!(value.tailscale_ip.is_empty());
        assert!(value.endpoint.is_empty());
    }
    let pending = local_worker_snapshot(&control, true, 19427, "synthetic-token".into());
    assert!(!pending.running);
    assert!(pending.endpoint.is_empty());
    let mut service = control.service.lock().unwrap();
    *service = Some(WorkerServiceHandle {
        stop: Arc::new(AtomicBool::new(false)),
        thread: None,
        bound_ip: "100.64.0.42".into(),
    });
    drop(service);
    assert_eq!(
        local_worker_snapshot(&control, true, 19427, "synthetic-token".into()).endpoint,
        "http://100.64.0.42:19427"
    );
}

#[test]
fn tailscale_probe_forces_cli_mode_and_preserves_app_cli_candidates() {
    let f = Fixture::new();
    let record = f.0.join("mode");
    let script = f.script(
        "fake-cli",
        &format!(
            "printf '%s|%s|%s' \"$TAILSCALE_BE_CLI\" \"$1\" \"$2\" > '{}'\nprintf '100.64.0.42\\n'",
            record.display()
        ),
    );
    assert_eq!(
        detect_tailscale_ip_with(&[script], Duration::from_millis(300)).as_deref(),
        Some("100.64.0.42")
    );
    assert_eq!(std::fs::read_to_string(record).unwrap(), "1|ip|-4");
    #[cfg(target_os = "macos")]
    assert!(tailscale_candidates()
        .iter()
        .any(|p| p == "/Applications/Tailscale.app/Contents/MacOS/Tailscale"));
}

#[test]
fn hanging_probe_and_inherited_capture_are_bounded_and_only_owned_child_is_stopped() {
    let f = Fixture::new();
    let owned_pid = f.0.join("owned-pid");
    let script = f.script(
        "fake-hang",
        &format!(
            "printf '%s' \"$$\" > '{}'\n/bin/sleep 10 &\nwait",
            owned_pid.display()
        ),
    );
    let started = Instant::now();
    assert!(detect_tailscale_ip_with(&[script], Duration::from_millis(100)).is_none());
    assert!(started.elapsed() < Duration::from_secs(2));
    let pid: i32 = std::fs::read_to_string(owned_pid).unwrap().parse().unwrap();
    assert_ne!(pid, std::process::id() as i32);
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
}

#[test]
fn successful_parent_with_grandchild_holding_stdout_cannot_wait_for_pipe_eof() {
    let f = Fixture::new();
    let script = f.script(
        "fake-inherited",
        "printf '100.64.0.42\\n'\n/bin/sleep 10 &\nexit 0",
    );
    let started = Instant::now();
    // Group cleanup may fail closed while macOS reaps an orphan zombie; it
    // must always return promptly instead of waiting for a ten-second pipe.
    let _ = detect_tailscale_ip_with(&[script], Duration::from_millis(100));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn candidates_share_one_deadline_and_non_tailnet_output_is_rejected() {
    let f = Fixture::new();
    let second_called = f.0.join("second-called");
    let slow = f.script("fake-first", "/bin/sleep 10");
    let second = f.script(
        "fake-second",
        &format!(
            "touch '{}'\nprintf '100.64.0.42\\n'",
            second_called.display()
        ),
    );
    assert!(detect_tailscale_ip_with(&[slow, second], Duration::from_millis(100)).is_none());
    assert!(!second_called.exists());
    for address in ["100.1.2.3", "100.not-an-ip", "127.0.0.1"] {
        let script = f.script("fake-invalid", &format!("printf '{address}\\n'"));
        assert!(detect_tailscale_ip_with(&[script], Duration::from_millis(100)).is_none());
    }
}
