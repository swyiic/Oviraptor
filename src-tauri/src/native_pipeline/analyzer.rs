//! §10.2 items 5-10 — one runner for every external analyzer. It owns the argument
//! array, the sandbox contract, the output caps, the provenance a result needs and the
//! dedup key that keeps a crash recovery from paying for the same scan twice.

use super::process::{self, ProcessLimits};
use crate::artifact_import::canonical::sha256_hex;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value as JsonValue};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalyzerEngine {
    Semgrep,
    CodeQL,
}

impl AnalyzerEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Semgrep => "semgrep",
            Self::CodeQL => "codeql",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "semgrep" => Some(Self::Semgrep),
            "codeql" => Some(Self::CodeQL),
            _ => None,
        }
    }
}

/// Anything but `Ran` is a coverage gap: an analyzer that was absent, that timed out or
/// that produced no SARIF may never be read as a clean result (§10.2 item 9, CODE-005).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnalyzerStatus {
    Ran { exit: Option<i32> },
    Missing { reason: String },
    TimedOut,
    Cancelled,
    Failed { exit: Option<i32>, detail: String },
}

impl AnalyzerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ran { .. } => "ran",
            Self::Missing { .. } => "missing",
            Self::TimedOut => "timeout",
            Self::Cancelled => "cancelled",
            Self::Failed { .. } => "failed",
        }
    }

    pub fn is_gap(&self) -> bool {
        !matches!(self, Self::Ran { .. })
    }

    pub fn gap_code(&self) -> &'static str {
        match self {
            Self::Missing { .. } => "analyzer_missing",
            Self::TimedOut => "analyzer_timeout",
            Self::Cancelled => "analyzer_cancelled",
            Self::Failed { .. } => "analyzer_failed",
            Self::Ran { .. } => "",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AnalyzerSpec {
    pub engine: AnalyzerEngine,
    /// Host binary, or the container runtime when `image` is set.
    pub program: PathBuf,
    pub image: Option<String>,
    pub rule_pack: PathBuf,
    pub languages: Vec<String>,
    pub repository: PathBuf,
    pub scratch_dir: PathBuf,
    pub scan_id: String,
    pub attempt_number: i64,
    pub limits: ProcessLimits,
}

#[derive(Clone, Debug)]
pub struct AnalyzerOutcome {
    pub engine: AnalyzerEngine,
    pub status: AnalyzerStatus,
    pub version: String,
    pub rule_pack_digest: String,
    pub image_digest: String,
    pub network_disabled: bool,
    pub repository_read_only: bool,
    pub args: Vec<Vec<String>>,
    pub sarif_path: Option<PathBuf>,
    pub sarif_sha256: String,
    pub truncated: bool,
    pub duration: Duration,
    pub invocation_key: String,
    /// True when this outcome was read back from an earlier run of the same snapshot
    /// rather than executed again — the crash-recovery path (§CODE-011).
    pub reused: bool,
}

impl AnalyzerOutcome {
    pub fn produced_results(&self) -> bool {
        !self.status.is_gap()
            && self.sarif_sha256.len() == 64
            && self.sarif_path.as_ref().is_some_and(|path| path.is_file())
    }

    pub fn gap_reason(&self) -> String {
        if self.status.is_gap() {
            return self.status.gap_code().to_string();
        }
        "no_sarif_output".to_string()
    }

    /// Provenance that travels with every evidence row (§10.2 item 8).
    pub fn evidence_json(&self) -> JsonValue {
        json!({
            "engine": self.engine.as_str(),
            "analyzerVersion": self.version,
            "rulePackDigest": self.rule_pack_digest,
            "imageDigest": self.image_digest,
            "networkDisabled": self.network_disabled,
            "repositoryReadOnly": self.repository_read_only,
            "status": self.status.as_str(),
            "invocationKey": self.invocation_key,
            "truncated": self.truncated,
            "durationMillis": self.duration.as_millis() as u64,
            "reused": self.reused,
            "sarifSha256": self.sarif_sha256,
        })
    }
}

/// Read a regular, bounded result once; the same bytes can be hashed and staged.
/// No symlink or special file may stand in for an analyzer output.
pub fn read_sarif(path: &Path) -> Result<Vec<u8>, String> {
    let limit = crate::artifact_import::limits::Limits::default().file_bytes;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|error| format!("sarif_read:{error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
        return Err("sarif_not_a_bounded_regular_file".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("sarif_open:{error}"))?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !opened.is_file() || opened.len() > limit {
        return Err("sarif_not_a_bounded_regular_file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("sarif_size_limit".into());
    }
    Ok(bytes)
}

/// The OS owns this lock for the lifetime of the file handle, including across DB
/// connections/processes. It is released on crash without stealing a live lease.
#[derive(Debug)]
struct InvocationOwner(std::fs::File);

impl Drop for InvocationOwner {
    fn drop(&mut self) {
        // Explicitly release the lock before closing the descriptor. In particular,
        // another invocation in this process may immediately open the same slot;
        // relying on descriptor teardown alone has intermittently left that next
        // invocation seeing a busy lock on macOS under parallel test execution.
        let _ = self.0.unlock();
    }
}

/// Lock the output slot (not only the rule digest) because different rule versions
/// of the same engine still write the same SARIF/database directory.
fn claim_output_slot(spec: &AnalyzerSpec) -> Result<InvocationOwner, String> {
    let path = spec
        .scratch_dir
        .join(format!(".{}-invocation.lock", spec.engine.as_str()));
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    if std::fs::symlink_metadata(&path)
        .is_ok_and(|metadata| !metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err("analyzer_invocation_lock_not_regular".into());
    }
    let lock = options
        .open(path)
        .map_err(|error| format!("analyzer_invocation_lock:{error}"))?;
    if !lock
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("analyzer_invocation_lock_not_regular".into());
    }
    lock.try_lock()
        .map_err(|error| format!("analyzer_invocation_in_progress_or_lock_unavailable:{error}"))?;
    Ok(InvocationOwner(lock))
}

pub fn image_digest(image: &str) -> Option<String> {
    let (name, digest) = image.rsplit_once('@')?;
    let body = digest.strip_prefix("sha256:")?;
    (!name.is_empty()
        && body.len() == 64
        && body.chars().all(|character| character.is_ascii_hexdigit()))
    .then(|| body.to_ascii_lowercase())
}

/// Digest of every rule file, in sorted path order, so a rule edit changes the identity
/// of the results and a re-run of the same pack does not.
pub fn rule_pack_digest_of(pack: &Path) -> String {
    let mut entries: Vec<PathBuf> = Vec::new();
    let mut stack = vec![pack.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            // A rule pack may also be handed over as one YAML file.
            entries.push(dir);
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                entries.push(path);
            }
        }
    }
    entries.sort();
    let mut joined = String::new();
    for path in entries {
        let relative = path
            .strip_prefix(pack)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = std::fs::read(&path).unwrap_or_default();
        joined.push_str(&format!("{relative}\u{1}{}\n", sha256_hex(&bytes)));
    }
    sha256_hex(joined.as_bytes())
}

/// One SARIF per engine per attempt: the scratch directory is already attempt-scoped,
/// so the file name never has to encode a hash that is computed after it.
pub fn sarif_path(spec: &AnalyzerSpec) -> PathBuf {
    spec.scratch_dir
        .join(format!("{}.sarif", spec.engine.as_str()))
}

impl AnalyzerSpec {
    fn container_prefix(&self, image: &str) -> Vec<String> {
        vec![
            "run".to_string(),
            "--rm".to_string(),
            "--pull=never".to_string(),
            "--network".to_string(),
            "none".to_string(),
            "--read-only".to_string(),
            "--cap-drop=ALL".to_string(),
            "--security-opt=no-new-privileges".to_string(),
            "--pids-limit=128".to_string(),
            "--tmpfs".to_string(),
            "/tmp:rw,noexec,nosuid,size=256m".to_string(),
            "-v".to_string(),
            format!("{}:/src:ro", self.repository.display()),
            "-v".to_string(),
            format!("{}:/out", self.scratch_dir.display()),
            "-v".to_string(),
            format!("{}:/rules:ro", self.rule_pack.display()),
            image.to_string(),
        ]
    }
}

fn shell_free(steps: &[Vec<String>]) -> Result<(), String> {
    for step in steps {
        for value in step {
            if value.contains('&') || value.contains(';') || value.contains("&&") {
                return Err(format!(
                    "分析器参数里出现 shell 控制字符（{value}），已拒绝执行：参数只允许数组传递"
                ));
            }
        }
    }
    Ok(())
}

/// The exact argv arrays this run would use, built once so the dedup key, the audit row
/// and the process all agree (§10.2 item 7).
pub fn planned_args(spec: &AnalyzerSpec) -> Result<Vec<Vec<String>>, String> {
    if let Some(image) = &spec.image {
        if image_digest(image).is_none() {
            return Err(format!(
                "分析器镜像必须用 digest 固定（{image}），否则同一份结果无法复现"
            ));
        }
    }
    let sarif = if spec.image.is_some() {
        format!("/out/{}.sarif", spec.engine.as_str())
    } else {
        sarif_path(spec).to_string_lossy().to_string()
    };
    let steps = match spec.engine {
        AnalyzerEngine::Semgrep => vec![vec![
            "scan".to_string(),
            "--config".to_string(),
            if spec.image.is_some() {
                "/rules".into()
            } else {
                spec.rule_pack.to_string_lossy().into()
            },
            "--sarif".to_string(),
            "--output".to_string(),
            sarif,
            if spec.image.is_some() {
                "/src".into()
            } else {
                spec.repository.to_string_lossy().into()
            },
        ]],
        AnalyzerEngine::CodeQL => {
            let database = if spec.image.is_some() {
                Path::new("/out")
            } else {
                &spec.scratch_dir
            }
            .join(format!(
                "codeql-{}.db",
                spec.languages.join("+").replace(['/', ' '], "_")
            ))
            .to_string_lossy()
            .to_string();
            vec![
                vec![
                    "database".to_string(),
                    "create".to_string(),
                    format!(
                        "--source-root={}",
                        if spec.image.is_some() {
                            Path::new("/src")
                        } else {
                            &spec.repository
                        }
                        .display()
                    ),
                    format!(
                        "--language={}",
                        spec.languages
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "python".into())
                    ),
                    "--overwrite".to_string(),
                    database.clone(),
                ],
                vec![
                    "database".to_string(),
                    "analyze".to_string(),
                    "--format=sarif-latest".to_string(),
                    "--output".to_string(),
                    sarif,
                    database,
                    if spec.image.is_some() {
                        "/rules".into()
                    } else {
                        spec.rule_pack.to_string_lossy().into()
                    },
                ],
            ]
        }
    };
    shell_free(&steps)?;
    let final_steps = match &spec.image {
        Some(image) => steps
            .into_iter()
            .map(|step| {
                let mut prefix = spec.container_prefix(image);
                prefix.extend(step);
                prefix
            })
            .collect(),
        None => steps,
    };
    // The container prefix is part of the argv too: a rule path with a shell
    // metacharacter in it must be refused here, not at the mount.
    shell_free(&final_steps)?;
    Ok(final_steps)
}

pub fn invocation_key(
    spec: &AnalyzerSpec,
    steps: &[Vec<String>],
    rule_pack_digest: &str,
) -> String {
    let joined = steps
        .iter()
        .map(|step| step.join("\u{1}"))
        .collect::<Vec<_>>()
        .join("\u{2}");
    sha256_hex(
        format!(
            "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
            spec.engine.as_str(),
            // Two different binaries given the same arguments are not the same run, and
            // a crash recovery must never read back the other one's result.
            spec.program.display(),
            spec.image.clone().unwrap_or_default(),
            rule_pack_digest,
            spec.scan_id,
            spec.attempt_number,
            joined
        )
        .as_bytes(),
    )
}

/// `(status, sarif path, version, network disabled, repository read only)`.
type StoredRun = (String, PathBuf, String, bool, bool);

/// The run stored under this key, if one exists. The two sandbox flags come back from
/// the row instead of being asserted, so a resume cannot claim more isolation than the
/// original run actually had.
pub fn stored_outcome(connection: &Connection, key: &str) -> Result<Option<StoredRun>, String> {
    connection
        .query_row(
            "SELECT status, sarif_path, version, network_disabled, repository_read_only
             FROM analyzer_runs WHERE invocation_key=?1",
            [key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    PathBuf::from(row.get::<_, String>(1)?),
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, i64>(4)? != 0,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("无法读取分析器执行记录：{error}"))
}

fn record_run(
    connection: &Connection,
    spec: &AnalyzerSpec,
    outcome: &AnalyzerOutcome,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO analyzer_runs(
                invocation_key,scan_id,attempt_number,engine,status,gap_code,version,
                rule_pack_digest,image_digest,network_disabled,repository_read_only,
                sarif_path,stdout_truncated,duration_millis,args_json,evidence_json,created_at
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,datetime('now','localtime'))
             ON CONFLICT(invocation_key) DO UPDATE SET
                status=excluded.status,gap_code=excluded.gap_code,version=excluded.version,
                rule_pack_digest=excluded.rule_pack_digest,image_digest=excluded.image_digest,
                network_disabled=excluded.network_disabled,repository_read_only=excluded.repository_read_only,
                sarif_path=excluded.sarif_path,stdout_truncated=excluded.stdout_truncated,
                duration_millis=excluded.duration_millis,args_json=excluded.args_json,
                evidence_json=excluded.evidence_json,created_at=excluded.created_at
             WHERE analyzer_runs.status!='ran'",
            rusqlite::params![
                outcome.invocation_key,
                spec.scan_id,
                spec.attempt_number,
                outcome.engine.as_str(),
                outcome.status.as_str(),
                outcome.status.gap_code(),
                outcome.version,
                outcome.rule_pack_digest,
                outcome.image_digest,
                i64::from(outcome.network_disabled),
                i64::from(outcome.repository_read_only),
                outcome
                    .sarif_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_default(),
                i64::from(outcome.truncated),
                outcome.duration.as_millis() as i64,
                serde_json::to_string(&outcome.args).unwrap_or_else(|_| "[]".into()),
                outcome.evidence_json().to_string(),
            ],
        )
        .map_err(|error| format!("无法登记分析器执行：{error}"))?;
    Ok(())
}

fn version_of(
    connection: &Connection,
    spec: &AnalyzerSpec,
    key: &str,
    cancelled: &dyn Fn() -> bool,
    logged: bool,
) -> Result<String, String> {
    let mut probe = spec
        .image
        .as_deref()
        .map(|image| spec.container_prefix(image))
        .unwrap_or_default();
    probe.extend(match spec.engine {
        AnalyzerEngine::Semgrep => vec!["--version".to_string()],
        AnalyzerEngine::CodeQL => vec!["version".to_string()],
    });
    let limits = ProcessLimits {
        timeout: spec.limits.timeout.min(Duration::from_secs(30)),
        stdout_bytes: 64 * 1024,
        stderr_bytes: 16 * 1024,
        ..ProcessLimits::default()
    };
    if spec.image.is_some() {
        let run = super::container::run_with_logs(
            connection,
            spec,
            key,
            "version",
            &probe,
            &limits,
            super::container::RunControl { cancelled, logged },
        )?;
        if !run.succeeded() {
            return Err("analyzer_version_probe_incomplete".into());
        }
        return Ok(run
            .stdout_text()
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string());
    }
    if logged {
        let run =
            process::log::driver::run(connection, spec, key, "version", &probe, &limits, cancelled)
                .map_err(|error| error.describe())?;
        if !run.succeeded() {
            return Err("analyzer_version_probe_incomplete".into());
        }
        return Ok(run
            .stdout_text()
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string());
    }
    Ok(process::run(
        &spec.program,
        &probe,
        Some(&spec.scratch_dir),
        &limits,
        cancelled,
    )
    .map(|run| {
        run.stdout_text()
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    })
    .unwrap_or_default())
}

/// Runs one analyzer over a frozen snapshot. A previous run of the same snapshot, engine
/// and rule pack is read back instead of executed again, which is what keeps a resume
/// from paying twice (CODE-011).
pub fn run(
    connection: &Connection,
    spec: &AnalyzerSpec,
    cancelled: &dyn Fn() -> bool,
) -> Result<AnalyzerOutcome, String> {
    run_inner(connection, spec, cancelled, false)
}

pub(crate) fn run_logged(
    connection: &Connection,
    spec: &AnalyzerSpec,
    cancelled: &dyn Fn() -> bool,
) -> Result<AnalyzerOutcome, String> {
    run_inner(connection, spec, cancelled, true)
}

fn run_inner(
    connection: &Connection,
    spec: &AnalyzerSpec,
    cancelled: &dyn Fn() -> bool,
    logged: bool,
) -> Result<AnalyzerOutcome, String> {
    std::fs::create_dir_all(&spec.scratch_dir).map_err(|error| error.to_string())?;
    let _invocation_owner = claim_output_slot(spec)?;
    super::container::ensure_no_pending(connection, &spec.scan_id)?;
    let rule_pack_digest = rule_pack_digest_of(&spec.rule_pack);
    let steps = planned_args(spec)?;
    let key = invocation_key(spec, &steps, &rule_pack_digest);
    let sarif = sarif_path(spec);
    let image_digest = spec
        .image
        .as_deref()
        .and_then(image_digest)
        .unwrap_or_default();
    if let Some((status, stored, version, network_disabled, repository_read_only)) =
        stored_outcome(connection, &key)?
    {
        if status == "ran" {
            if stored != sarif {
                return Err("analyzer_cached_result_path_mismatch".into());
            }
            let evidence: String = connection
                .query_row(
                    "SELECT evidence_json FROM analyzer_runs WHERE invocation_key=?1",
                    [&key],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            let evidence: JsonValue =
                serde_json::from_str(&evidence).map_err(|_| "analyzer_cached_evidence_invalid")?;
            let expected = evidence["sarifSha256"]
                .as_str()
                .filter(|hash| hash.len() == 64)
                .ok_or("analyzer_cached_result_unverified")?;
            if sha256_hex(&read_sarif(&stored)?) != expected {
                return Err("analyzer_cached_result_hash_mismatch".into());
            }
            return Ok(AnalyzerOutcome {
                engine: spec.engine,
                status: AnalyzerStatus::Ran { exit: Some(0) },
                version,
                rule_pack_digest,
                image_digest,
                network_disabled,
                repository_read_only,
                args: steps,
                sarif_path: Some(stored),
                sarif_sha256: expected.into(),
                truncated: false,
                duration: Duration::ZERO,
                invocation_key: key,
                reused: true,
            });
        }
    }
    // Preserve a previous unaccepted output for diagnosis, but never let a success
    // which emitted no file relabel it as this execution's result.
    match std::fs::symlink_metadata(&sarif) {
        Ok(_) => std::fs::rename(
            &sarif,
            spec.scratch_dir.join(format!(
                "{}.stale-{}",
                spec.engine.as_str(),
                uuid::Uuid::new_v4()
            )),
        )
        .map_err(|error| format!("analyzer_stale_output_isolation:{error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("analyzer_output_inspection:{error}")),
    }
    let version = version_of(connection, spec, &key, cancelled, logged)?;
    let mut duration = Duration::ZERO;
    let mut truncated = false;
    let mut status = AnalyzerStatus::Failed {
        exit: None,
        detail: "该引擎没有可执行的步骤".into(),
    };
    for (index, step) in steps.iter().enumerate() {
        if cancelled() {
            status = AnalyzerStatus::Cancelled;
            break;
        }
        let execution = if spec.image.is_some() {
            super::container::run_with_logs(
                connection,
                spec,
                &key,
                &format!("step-{index}"),
                step,
                &spec.limits,
                super::container::RunControl { cancelled, logged },
            )
            .map_err(|reason| process::ProcessError::Launch {
                program: spec.program.display().to_string(),
                reason,
            })
        } else if logged {
            process::log::driver::run(
                connection,
                spec,
                &key,
                &format!("step-{index}"),
                step,
                &spec.limits,
                cancelled,
            )
        } else {
            process::run(
                &spec.program,
                step,
                Some(&spec.scratch_dir),
                &spec.limits,
                cancelled,
            )
        };
        match execution {
            Ok(run) => {
                duration += run.duration;
                truncated |= run.truncated;
                if run.timed_out {
                    status = AnalyzerStatus::TimedOut;
                    break;
                }
                if run.cancelled {
                    status = AnalyzerStatus::Cancelled;
                    break;
                }
                if !run.succeeded() {
                    let tail = run
                        .cleanup_error
                        .clone()
                        .unwrap_or_else(|| run.stderr_text().chars().take(400).collect::<String>());
                    status = AnalyzerStatus::Failed {
                        exit: run.exit,
                        detail: tail,
                    };
                    break;
                }
                status = AnalyzerStatus::Ran { exit: run.exit };
            }
            Err(error) => {
                status = if error.is_missing() {
                    AnalyzerStatus::Missing {
                        reason: error.describe(),
                    }
                } else {
                    AnalyzerStatus::Failed {
                        exit: None,
                        detail: error.describe(),
                    }
                };
                break;
            }
        }
    }
    let sarif_sha256 = if !status.is_gap() && sarif.exists() {
        sha256_hex(&read_sarif(&sarif)?)
    } else {
        String::new()
    };
    let outcome = AnalyzerOutcome {
        engine: spec.engine,
        status,
        version,
        rule_pack_digest,
        image_digest,
        // Only a pinned container run actually holds the mount and the network policy; a
        // host binary says so, and the caller turns that into a visible gap.
        network_disabled: spec.image.is_some(),
        repository_read_only: spec.image.is_some(),
        args: steps,
        sarif_path: sarif.is_file().then_some(sarif),
        sarif_sha256,
        truncated,
        duration,
        invocation_key: key,
        reused: false,
    };
    record_run(connection, spec, &outcome)?;
    Ok(outcome)
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn output_slot_excludes_a_live_owner_and_releases_for_the_next_run() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-analyzer-lock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let spec = AnalyzerSpec {
            engine: AnalyzerEngine::Semgrep,
            program: root.join("unused"),
            image: None,
            rule_pack: root.join("unused"),
            languages: Vec::new(),
            repository: root.join("unused"),
            scratch_dir: root.clone(),
            scan_id: "lock-test".into(),
            attempt_number: 1,
            limits: ProcessLimits::default(),
        };
        let owner = claim_output_slot(&spec).unwrap();
        assert!(claim_output_slot(&spec)
            .unwrap_err()
            .contains("analyzer_invocation_in_progress"));
        drop(owner);
        let successor = claim_output_slot(&spec).unwrap();
        drop(successor);
        std::fs::remove_dir_all(root).unwrap();
    }
}
