use super::*;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn temporary_root() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "oviraptor-owned-host-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}

fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

fn assert_gone(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while alive(pid) && Instant::now() < deadline {
        thread::sleep(POLL);
    }
    assert!(!alive(pid), "owned fixture PID {pid} survived shutdown");
}

fn host_fixture(mode: &str) -> (OwnedHostProcess, u32, PathBuf) {
    let root = temporary_root();
    let script = root.join("host.mjs");
    fs::write(&script, r#"
import {spawn} from 'node:child_process';
import {writeFileSync} from 'node:fs';
const [root,mode]=process.argv.slice(2);
const payload=(mode==='normal'?'':"process.on('SIGTERM',()=>{});")+"console.log('ready');setInterval(()=>{},1000);";
const worker=spawn(process.execPath,['-e',payload],{stdio:['ignore','pipe','ignore']});
worker.stdout.once('data',()=>{writeFileSync(root+'/worker.pid',String(worker.pid));console.log('ready');});
process.on('SIGTERM',()=>{if(mode==='normal'){worker.kill();setTimeout(()=>process.exit(0),20);}else if(mode==='orphan'){process.exit(0);}});
setInterval(()=>{},1000);
"#).unwrap();
    let mut command =
        Command::new(std::env::var_os("OVIRAPTOR_TEST_NODE").unwrap_or_else(|| "node".into()));
    command
        .arg(&script)
        .arg(&root)
        .arg(mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut owned = OwnedHostProcess::spawn(&mut command).unwrap();
    owned.host_grace = Duration::from_millis(if mode == "normal" { 500 } else { 120 });
    let mut line = String::new();
    BufReader::new(owned.take_stdout().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "ready");
    let worker = fs::read_to_string(root.join("worker.pid"))
        .unwrap()
        .parse()
        .unwrap();
    (owned, worker, root)
}

#[test]
fn graceful_host_shutdown_reaps_its_worker_and_is_idempotent() {
    let (mut owned, worker, root) = host_fixture("normal");
    let host = owned.child.id();
    assert_ne!(owned.group, unsafe { libc::getpgrp() });
    let outcome = owned.stop();
    assert!(
        !outcome.forced,
        "idle JS-style shutdown should remain graceful"
    );
    assert!(outcome.host_reaped);
    assert_gone(host);
    assert_gone(worker);
    assert!(owned.stop().host_reaped);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stubborn_owned_tree_is_forced_without_signalling_an_unrelated_group() {
    let (mut unrelated, unrelated_worker, unrelated_root) = host_fixture("normal");
    let (mut owned, worker, root) = host_fixture("stubborn");
    let host = owned.child.id();
    let outcome = owned.stop();
    assert!(outcome.forced);
    assert!(outcome.host_reaped);
    assert_gone(host);
    assert_gone(worker);
    assert!(alive(unrelated.child.id()));
    assert!(alive(unrelated_worker));
    unrelated.stop();
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(unrelated_root).unwrap();
}

#[test]
fn host_exiting_first_does_not_leave_its_worker_running() {
    let (mut owned, worker, root) = host_fixture("orphan");
    let outcome = owned.stop();
    assert!(outcome.forced);
    assert!(outcome.host_reaped);
    assert_gone(worker);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dropping_a_startup_guard_cleans_the_entire_owned_tree() {
    let (owned, worker, root) = host_fixture("stubborn");
    let host = owned.child.id();
    drop(owned); // Same path as an invalid ready response / startup error.
    assert_gone(host);
    assert_gone(worker);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn parent_sigterm_routes_through_the_exit_callback_in_an_isolated_process() {
    let root = temporary_root();
    let child_test = format!(
        "{}::signal_bridge_child",
        module_path!()
            .split("::")
            .skip(1)
            .collect::<Vec<_>>()
            .join("::")
    );
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &child_test, "--ignored", "--nocapture"])
        .env("OVIRAPTOR_SIGNAL_TEST_ROOT", &root)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !root.join("ready").is_file() && Instant::now() < deadline {
        thread::sleep(POLL);
    }
    if !root.join("ready").is_file() {
        let _ = child.kill();
        panic!("isolated signal fixture did not become ready");
    }
    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(root.join("signal")).unwrap(),
        libc::SIGTERM.to_string()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "only run as an isolated child by the parent signal regression"]
fn signal_bridge_child() {
    let Some(root) = std::env::var_os("OVIRAPTOR_SIGNAL_TEST_ROOT").map(PathBuf::from) else {
        return;
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let bridge = ExitSignalBridge::install(move |signal| {
        sender.send(signal).unwrap();
    })
    .unwrap();
    fs::write(root.join("ready"), "ready").unwrap();
    let signal = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
    fs::write(root.join("signal"), signal.to_string()).unwrap();
    drop(bridge);
}
