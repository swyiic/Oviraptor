//! macOS Seatbelt (`sandbox-exec`) lightweight sandbox for running a *static*,
//! non-executing analyzer over untrusted downloaded source — the no-Docker
//! alternative to the container runner in `native_pipeline/analyzer.rs`.
//!
//! Threat model: the source is DATA that the analyzer reads and parses, never
//! code we execute. So the two axes that actually matter are (1) exfiltration
//! and (2) tampering. The Seatbelt profile closes both — `(deny network*)` and
//! writes confined to the scratch directory — while leaving reads open so the
//! analyzer can run unmodified. CPU/address-space are bounded with rlimits and a
//! wall-clock timeout; an oversized checkout is rejected before launch. This is
//! NOT a proven hard memory boundary (Seatbelt has none; see
//! `require_supported`'s honest refusal), so it is only ever used for analyzers
//! that never execute the target code (e.g. Semgrep), never for a build-running
//! engine (e.g. CodeQL database-create on a compiled language).
//!
//! The runner below is a verified primitive that is not yet wired into the
//! production analyzer pipeline (that flip — turning host-binary runs into
//! isolated runs and setting the isolation flags — is a reviewed follow-up), so
//! its surface is intentionally dead until then.
#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(crate) struct MacosStaticParserReadiness {
    mechanism_available: bool,
}
impl MacosStaticParserReadiness {
    pub(crate) fn inspect() -> Self {
        #[cfg(target_os = "macos")]
        let available = {
            use std::os::unix::fs::MetadataExt;
            std::fs::symlink_metadata("/usr/bin/sandbox-exec")
                .is_ok_and(|m| m.is_file() && m.uid() == 0 && m.mode() & 0o022 == 0)
        };
        #[cfg(not(target_os = "macos"))]
        let available = false;
        Self {
            mechanism_available: available,
        }
    }
    /// Available means the FS+network isolation mechanism is present and
    /// untampered. It deliberately does NOT claim a hard memory boundary: the
    /// caller gets FS+network containment plus rlimit/timeout best-effort, and
    /// must only run non-executing analyzers under it. Callers that require a
    /// proven memory cgroup must use the container runner instead.
    pub(crate) fn mechanism_available(&self) -> bool {
        self.mechanism_available
    }
    /// Unchanged from the original scaffold: callers that demand a proven hard
    /// memory boundary (which Seatbelt cannot give) are still refused here and
    /// steered to the container runner. `run_sandboxed` uses the weaker
    /// `mechanism_available` instead, because for a non-executing analyzer the
    /// FS+network containment plus rlimit/timeout best-effort is proportionate.
    pub(crate) fn require_supported(&self) -> Result<(), &'static str> {
        if !self.mechanism_available {
            return Err("unsupported_sandbox:macos_seatbelt_unavailable");
        }
        // RLIMIT_CPU is CPU time, not a core quota. RLIMIT_RSS/AS must not
        // silently stand in for an unproved hard per-run memory boundary.
        Err("unsupported_sandbox:macos_hard_memory_containment_unverified")
    }
}

/// Resource bounds applied to a sandboxed run. These are best-effort limits, not
/// a proven containment boundary; `max_source_bytes` is the primary defense
/// against a pathological checkout and is enforced before launch.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SandboxLimits {
    pub address_space_bytes: u64,
    pub cpu_seconds: u64,
    pub wall_seconds: u64,
    pub max_source_bytes: u64,
    pub max_output_bytes: usize,
}
impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            address_space_bytes: 4 * 1024 * 1024 * 1024, // 4 GiB
            cpu_seconds: 900,
            wall_seconds: 1200,
            max_source_bytes: 2 * 1024 * 1024 * 1024, // 2 GiB checkout cap
            max_output_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub(crate) struct SandboxRun {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
    pub duration: Duration,
    /// True only because the profile below actually carries `(deny network*)`.
    pub network_disabled: bool,
    /// True only because the profile denies writes outside the scratch dir, so
    /// the read-only source mount cannot be modified.
    pub source_read_only: bool,
}

/// Build the Seatbelt profile. The model is allow-read / deny-network /
/// write-only-scratch: reading is not the threat once the network is closed, so
/// the analyzer keeps working while nothing it reads can leave the host and
/// nothing outside `scratch` can be modified. A stricter `(deny default)`
/// profile is a hardening follow-up; it must be tuned against each analyzer's
/// real syscall needs or it silently breaks the run.
fn seatbelt_profile(scratch: &Path) -> String {
    // scratch is an absolute, canonicalized path we created; it contains no
    // profile metacharacters because we reject non-simple paths before here.
    format!(
        "(version 1)\n\
         (allow default)\n\
         (deny network*)\n\
         (deny file-write*)\n\
         (allow file-write* (subpath \"{scratch}\"))\n\
         (allow file-write-data (literal \"/dev/null\") (literal \"/dev/stdout\") (literal \"/dev/stderr\") (literal \"/dev/dtracehelper\"))\n\
         (allow file-write-data (regex #\"^/dev/tty\"))\n",
        scratch = scratch.display()
    )
}

#[derive(Debug)]
pub(crate) enum SandboxError {
    Unavailable(&'static str),
    SourceTooLarge { bytes: u64, cap: u64 },
    BadPath(String),
    Spawn(String),
    /// The egress allowlist for a target-scoped network run was empty, or carried
    /// an address the sandbox profile cannot pin deterministically (v1: IPv4 only).
    EgressInvalid(String),
}
impl SandboxError {
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Unavailable(reason) => (*reason).to_string(),
            Self::SourceTooLarge { bytes, cap } => {
                format!("sandbox_source_too_large:{bytes}>{cap}")
            }
            Self::BadPath(path) => format!("sandbox_path_rejected:{path}"),
            Self::Spawn(reason) => format!("sandbox_spawn_failed:{reason}"),
            Self::EgressInvalid(reason) => format!("sandbox_egress_invalid:{reason}"),
        }
    }
}

/// Build the target-scoped egress profile. Seatbelt's `remote ip` grammar accepts
/// only `*` or `localhost` for the host (a specific IP literal is REJECTED by
/// sandbox-exec — empirically verified), so egress cannot be pinned to the target's
/// IP directly. Instead this re-allows network-outbound ONLY to `localhost:<port>`
/// for each given port after the blanket `(deny network*)`; the caller runs a
/// deterministic forward-proxy on that localhost port that connects ONLY to the
/// vetted target. So code inside the sandbox can reach exactly that one localhost
/// port — and through it only the target — and nothing else: other hosts, other
/// ports and DNS (udp/53, never re-allowed) are all denied. Seatbelt is last-match-
/// wins, so `localhost:<port>` matches the deny and this allow (allow wins) while
/// every other destination matches only the deny. Per-port enforcement on localhost
/// is empirically verified (see the macOS enforcement test).
fn seatbelt_egress_profile(scratch: &Path, localhost_ports: &[u16]) -> String {
    let mut allow = String::new();
    for port in localhost_ports {
        allow.push_str(&format!(
            "(allow network-outbound (remote ip \"localhost:{port}\"))\n"
        ));
    }
    format!(
        "(version 1)\n\
         (allow default)\n\
         (deny network*)\n\
         {allow}\
         (deny file-write*)\n\
         (allow file-write* (subpath \"{scratch}\"))\n\
         (allow file-write-data (literal \"/dev/null\") (literal \"/dev/stdout\") (literal \"/dev/stderr\") (literal \"/dev/dtracehelper\"))\n\
         (allow file-write-data (regex #\"^/dev/tty\"))\n",
        scratch = scratch.display()
    )
}

/// A path is usable in the profile / as a mount only when it is absolute and
/// free of the quoting and newline characters that could break out of the
/// profile's string literal.
fn simple_abs_path(path: &Path) -> Result<PathBuf, SandboxError> {
    let canonical = path
        .canonicalize()
        .map_err(|e| SandboxError::BadPath(format!("{}:{e}", path.display())))?;
    let text = canonical.to_string_lossy();
    if !canonical.is_absolute()
        || text.contains('"')
        || text.contains('\\')
        || text.contains('\n')
        || text.contains('\r')
    {
        return Err(SandboxError::BadPath(text.into_owned()));
    }
    Ok(canonical)
}

/// Total size of a directory tree, following no symlinks, so a planted symlink
/// to a huge file cannot inflate (or be followed out of) the checkout.
fn tree_size(root: &Path, cap: u64) -> u64 {
    fn walk(dir: &Path, acc: &mut u64, cap: u64) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if *acc > cap {
                return;
            }
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            let kind = meta.file_type();
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                walk(&entry.path(), acc, cap);
            } else if kind.is_file() {
                *acc = acc.saturating_add(meta.len());
            }
        }
    }
    let mut total = 0u64;
    walk(root, &mut total, cap);
    total
}

#[cfg(unix)]
fn apply_rlimits(limits: &SandboxLimits) -> std::io::Result<()> {
    // Runs in the forked child before exec. setrlimit is a plain syscall and is
    // safe to call here.
    // `resource` infers from setrlimit: c_int on macOS, __rlimit_resource_t (u32)
    // on Linux — so the libc::RLIMIT_* constants and setrlimit agree on both.
    let set = |resource, value: u64| -> std::io::Result<()> {
        let rl = libc::rlimit {
            rlim_cur: value as libc::rlim_t,
            rlim_max: value as libc::rlim_t,
        };
        // SAFETY: rl is a fully initialized rlimit for a valid resource id.
        if unsafe { libc::setrlimit(resource, &rl) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    };
    // CPU time is honoured across unixes; enforce it hard.
    set(libc::RLIMIT_CPU, limits.cpu_seconds)?;
    // RLIMIT_AS is best-effort only. macOS frequently rejects it with EINVAL
    // (and never gave a reliable address-space bound anyway — the honest
    // position in `require_supported`), so a failure here must NOT abort the
    // launch. The real defenses against a pathological run are the pre-launch
    // source-size cap and the wall-clock timeout, not this.
    if limits.address_space_bytes > 0 {
        let _ = set(libc::RLIMIT_AS, limits.address_space_bytes);
    }
    Ok(())
}

/// Run `program args...` over `source_dir` under Seatbelt, writing only into
/// `scratch_dir`, with the network denied and resource bounds applied. The
/// caller is responsible for only passing a non-executing analyzer.
pub(crate) fn run_sandboxed(
    program: &Path,
    args: &[String],
    source_dir: &Path,
    scratch_dir: &Path,
    limits: &SandboxLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<SandboxRun, SandboxError> {
    run_sandboxed_core(
        program,
        args,
        source_dir,
        scratch_dir,
        limits,
        cancelled,
        &|scratch| seatbelt_profile(scratch),
        true,
    )
}

/// Run `program args...` under Seatbelt with network-outbound RE-ALLOWED only to
/// `localhost:<port>` for each `localhost_ports` entry, and denied everywhere else
/// including DNS. This is the #6 target-scoped network sandbox: the caller runs a
/// deterministic forward-proxy on that localhost port that connects ONLY to the
/// vetted target, so agent-written PoC code can reach the authorized target — and
/// ONLY it — by talking to the local proxy; it can resolve no names and reach no
/// other host or port. (Seatbelt's `remote ip` rejects a specific target IP, so the
/// localhost-proxy indirection is how egress is pinned deterministically.)
/// SECURITY-CRITICAL: per-port localhost enforcement is verified by the macOS
/// enforcement test, but enabling this against a real target still needs a live
/// check that the proxy forwards to the intended host only.
pub(crate) fn run_sandboxed_egress(
    program: &Path,
    args: &[String],
    source_dir: &Path,
    scratch_dir: &Path,
    localhost_ports: &[u16],
    limits: &SandboxLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<SandboxRun, SandboxError> {
    if localhost_ports.is_empty() {
        return Err(SandboxError::EgressInvalid("empty_allowlist".into()));
    }
    let ports = localhost_ports.to_vec();
    run_sandboxed_core(
        program,
        args,
        source_dir,
        scratch_dir,
        limits,
        cancelled,
        &move |scratch| seatbelt_egress_profile(scratch, &ports),
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_sandboxed_core(
    program: &Path,
    args: &[String],
    source_dir: &Path,
    scratch_dir: &Path,
    limits: &SandboxLimits,
    cancelled: &dyn Fn() -> bool,
    make_profile: &dyn Fn(&Path) -> String,
    network_disabled: bool,
) -> Result<SandboxRun, SandboxError> {
    let readiness = MacosStaticParserReadiness::inspect();
    if !readiness.mechanism_available() {
        return Err(SandboxError::Unavailable(
            "unsupported_sandbox:macos_seatbelt_unavailable",
        ));
    }
    let source = simple_abs_path(source_dir)?;
    let scratch = simple_abs_path(scratch_dir)?;

    let size = tree_size(&source, limits.max_source_bytes);
    if size > limits.max_source_bytes {
        return Err(SandboxError::SourceTooLarge {
            bytes: size,
            cap: limits.max_source_bytes,
        });
    }

    let profile = make_profile(&scratch);
    let profile_path = scratch.join(".seatbelt-static-parser.sb");
    std::fs::write(&profile_path, profile)
        .map_err(|e| SandboxError::Spawn(format!("profile_write:{e}")))?;

    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .arg("-f")
        .arg(&profile_path)
        .arg(program)
        .args(args)
        .current_dir(&scratch)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", &scratch)
        .env("TMPDIR", &scratch)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let limits = *limits;
        // SAFETY: only async-signal-safe syscalls (setrlimit) run here.
        unsafe {
            command.pre_exec(move || apply_rlimits(&limits));
        }
    }

    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|e| SandboxError::Spawn(e.to_string()))?;

    // Drain stdout/stderr on their own threads so a chatty analyzer cannot
    // deadlock on a full pipe while we poll for the timeout.
    let cap = limits.max_output_bytes;
    let mut out_handle = child.stdout.take().map(|pipe| drain(pipe, cap));
    let mut err_handle = child.stderr.take().map(|pipe| drain(pipe, cap));

    let wall = Duration::from_secs(limits.wall_seconds.max(1));
    let mut timed_out = false;
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {}
            Err(e) => return Err(SandboxError::Spawn(format!("wait:{e}"))),
        }
        if started.elapsed() >= wall || cancelled() {
            timed_out = started.elapsed() >= wall;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    let (stdout, out_trunc) = out_handle
        .take()
        .map(|h| h.join().unwrap_or((Vec::new(), false)))
        .unwrap_or((Vec::new(), false));
    let (stderr, err_trunc) = err_handle
        .take()
        .map(|h| h.join().unwrap_or((Vec::new(), false)))
        .unwrap_or((Vec::new(), false));

    Ok(SandboxRun {
        exit_code,
        timed_out,
        stdout,
        stderr,
        truncated: out_trunc || err_trunc,
        duration: started.elapsed(),
        network_disabled,
        source_read_only: true,
    })
}

fn drain(
    mut pipe: impl Read + Send + 'static,
    cap: usize,
) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut truncated = false;
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if buf.len() < cap {
                        let room = cap - buf.len();
                        buf.extend_from_slice(&chunk[..n.min(room)]);
                        if n > room {
                            truncated = true;
                        }
                    } else {
                        truncated = true;
                    }
                }
                Err(_) => break,
            }
        }
        (buf, truncated)
    })
}

#[cfg(test)]
mod egress_profile_tests {
    use super::*;

    #[test]
    fn profile_allows_only_the_localhost_ports_and_denies_dns() {
        let scratch = Path::new("/tmp/ovr-scratch-egress");
        let p = seatbelt_egress_profile(scratch, &[8443, 9080]);
        assert!(p.contains("(deny network*)"));
        assert!(p.contains("(allow network-outbound (remote ip \"localhost:8443\"))"));
        assert!(p.contains("(allow network-outbound (remote ip \"localhost:9080\"))"));
        // DNS (udp/53) is never re-allowed, so names cannot be resolved inside.
        assert!(!p.contains(":53"));
        // deny precedes the allows (Seatbelt last-match-wins → allow for those ports only).
        let deny = p.find("(deny network*)").unwrap();
        let allow = p.find("remote ip").unwrap();
        assert!(deny < allow);
    }

    #[test]
    fn egress_rejects_empty_allowlist() {
        let here = Path::new(".");
        let noop = || false;
        // Empty port list is refused before the sandbox mechanism is consulted.
        assert!(matches!(
            run_sandboxed_egress(Path::new("/bin/echo"), &[], here, here, &[], &SandboxLimits::default(), &noop),
            Err(SandboxError::EgressInvalid(_))
        ));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod seatbelt_isolation_tests {
    use super::*;

    fn tmp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ovr-sbtest-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn skip_if_unavailable() -> bool {
        if !MacosStaticParserReadiness::inspect().mechanism_available() {
            eprintln!("seatbelt unavailable on this host; skipping");
            return true;
        }
        false
    }
    fn never() -> bool {
        false
    }

    #[test]
    fn reads_source_but_cannot_write_outside_scratch() {
        if skip_if_unavailable() {
            return;
        }
        let source = tmp("src");
        let scratch = tmp("scratch");
        std::fs::write(source.join("code.txt"), b"fn main() {}").unwrap();

        // Allowed: read a source file and write the copy into scratch.
        let run = run_sandboxed(
            Path::new("/bin/cp"),
            &[
                source.join("code.txt").to_string_lossy().into_owned(),
                scratch.join("copy.txt").to_string_lossy().into_owned(),
            ],
            &source,
            &scratch,
            &SandboxLimits::default(),
            &never,
        )
        .unwrap();
        assert_eq!(run.exit_code, Some(0), "read src + write scratch must work");
        assert!(scratch.join("copy.txt").is_file());

        // Denied: writing back into the source tree must fail (read-only source).
        let escape = run_sandboxed(
            Path::new("/bin/cp"),
            &[
                source.join("code.txt").to_string_lossy().into_owned(),
                source.join("injected.txt").to_string_lossy().into_owned(),
            ],
            &source,
            &scratch,
            &SandboxLimits::default(),
            &never,
        )
        .unwrap();
        assert_ne!(escape.exit_code, Some(0), "writing into source must be denied");
        assert!(
            !source.join("injected.txt").exists(),
            "source tree must stay unmodified"
        );
    }

    #[test]
    fn egress_reaches_the_allowed_localhost_port_and_nothing_else() {
        if skip_if_unavailable() {
            return;
        }
        if !Path::new("/usr/bin/python3").exists() {
            eprintln!("no /usr/bin/python3; skipping egress enforcement test");
            return;
        }
        use std::io::Write as _;
        use std::net::TcpListener;
        // Two loopback listeners that accept and immediately close, so a permitted
        // connect succeeds. The profile will allow ONLY the first port.
        let accept_loop = |listener: TcpListener| {
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(mut s) => {
                            let _ = s.write_all(b"ok");
                        }
                        Err(_) => break,
                    }
                }
            });
        };
        let la = TcpListener::bind("127.0.0.1:0").unwrap();
        let ld = TcpListener::bind("127.0.0.1:0").unwrap();
        let allow_port = la.local_addr().unwrap().port();
        let deny_port = ld.local_addr().unwrap().port();
        accept_loop(la);
        accept_loop(ld);

        let scratch = tmp("egress");
        let client = format!(
            "import socket\n\
             def probe(p):\n\
             \x20   try:\n\
             \x20       c=socket.create_connection(('127.0.0.1',p),timeout=3); c.recv(4); c.close(); return 'OK'\n\
             \x20   except Exception as e:\n\
             \x20       return 'REFUSED:'+type(e).__name__\n\
             print('ALLOW='+probe({allow_port}))\n\
             print('DENY='+probe({deny_port}))\n"
        );
        std::fs::write(scratch.join("client.py"), client).unwrap();

        let mut limits = SandboxLimits::default();
        limits.wall_seconds = 20;
        let run = run_sandboxed_egress(
            Path::new("/usr/bin/python3"),
            &["client.py".to_string()],
            &scratch,
            &scratch,
            &[allow_port],
            &limits,
            &never,
        )
        .expect("sandboxed egress run");
        let out = String::from_utf8_lossy(&run.stdout);
        assert!(!run.network_disabled, "egress run reports network enabled");
        assert!(out.contains("ALLOW=OK"), "allowed localhost port must connect; got: {out}\nstderr: {}", String::from_utf8_lossy(&run.stderr));
        assert!(out.contains("DENY=REFUSED"), "non-allowed localhost port must be refused; got: {out}");
    }

    #[test]
    fn rejects_oversized_checkout_before_launch() {
        if skip_if_unavailable() {
            return;
        }
        let source = tmp("big");
        let scratch = tmp("big-scratch");
        std::fs::write(source.join("blob"), vec![0u8; 4096]).unwrap();
        let mut limits = SandboxLimits::default();
        limits.max_source_bytes = 1024; // smaller than the blob
        let err = run_sandboxed(
            Path::new("/bin/cp"),
            &["x".into(), "y".into()],
            &source,
            &scratch,
            &limits,
            &never,
        );
        assert!(matches!(err, Err(SandboxError::SourceTooLarge { .. })));
    }
}
