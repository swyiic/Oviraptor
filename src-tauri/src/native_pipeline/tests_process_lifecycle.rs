//! No target traffic: subprocesses are local, short-lived fixture scripts.
use super::*;
use crate::native_pipeline::process;
use std::time::Instant;

#[test]
fn precancelled_process_does_not_even_try_to_launch() {
    let result = process::run(
        Path::new("/nonexistent/native-process-must-not-launch"),
        &[],
        None,
        &ProcessLimits::default(),
        &|| true,
    )
    .expect("pre-cancellation must win over executable lookup");
    assert!(result.cancelled);
    assert!(!result.succeeded());
}

#[test]
fn exhausted_deadline_does_not_even_try_to_launch() {
    let result = process::run(
        Path::new("/nonexistent/native-process-must-not-launch"),
        &[],
        None,
        &ProcessLimits {
            timeout: Duration::ZERO,
            ..Default::default()
        },
        &|| false,
    )
    .expect("zero remaining budget must not launch a program");
    assert!(result.timed_out);
}

fn descendant_fixture(mode: &str) {
    let root = sandbox(mode);
    fs::create_dir_all(&root).unwrap();
    // The background child inherits both pipes and would modify scratch after return.
    let ending = if mode == "success" { "exit 0" } else { "wait" };
    let script = format!("(sleep 1; printf escaped > escaped) & printf ready; {ending}");
    let start = Instant::now();
    let limits = ProcessLimits {
        timeout: if mode == "timeout" {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(3)
        },
        // Poll must never postpone the deadline by an arbitrary caller-supplied interval.
        poll: Duration::from_secs(2),
        ..Default::default()
    };
    let result = process::run(
        Path::new("/bin/sh"),
        &["-c".into(), script],
        Some(&root),
        &limits,
        &|| mode == "cancel" && start.elapsed() >= Duration::from_millis(50),
    )
    .unwrap();
    assert!(
        start.elapsed() < Duration::from_millis(700),
        "descendant pipes delayed termination: {:?}",
        start.elapsed()
    );
    assert_eq!(result.stdout_text(), "ready");
    match mode {
        "success" => assert!(result.succeeded()),
        "timeout" => assert!(result.timed_out),
        "cancel" => assert!(result.cancelled),
        _ => unreachable!(),
    }
    std::thread::sleep(Duration::from_millis(1100));
    assert!(
        !root.join("escaped").exists(),
        "a descendant continued modifying scratch after return"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn normal_exit_reaps_remaining_process_group_and_does_not_wait_for_its_pipes() {
    descendant_fixture("success");
}

#[test]
fn timeout_stops_descendants_and_preserves_partial_output() {
    descendant_fixture("timeout");
}

#[test]
fn cancellation_stops_descendants_and_preserves_partial_output() {
    descendant_fixture("cancel");
}
