//! Daemon-side lifecycle for pinned Native analyzers. Create is separate from start:
//! a lost create response can leave an inert container, never a running analyzer.
//! Ownership is verified before every destructive cleanup operation. A durable
//! unresolved receipt blocks retries instead of pretending cleanup succeeded.
use super::analyzer::AnalyzerSpec;
use super::process::{self, BoundedRun, ProcessLimits};
use rusqlite::{params, Connection};
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

const OWNER_LABEL: &str = "io.oviraptor.native-owner";

pub(super) fn ensure_no_pending(connection: &Connection, scan_id: &str) -> Result<(), String> {
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM analyzer_container_receipts WHERE scan_id=?1 AND cleanup_status<>'confirmed'",
        [scan_id], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if count > 0 {
        return Err("analyzer_cleanup_unresolved:inspect_persisted_receipts_before_retry".into());
    }
    Ok(())
}

fn daemon(spec: &AnalyzerSpec, args: &[String]) -> Result<BoundedRun, String> {
    process::run(
        &spec.program,
        args,
        Some(&spec.scratch_dir),
        &ProcessLimits {
            timeout: Duration::from_secs(5),
            stdout_bytes: 64 * 1024,
            stderr_bytes: 16 * 1024,
            ..Default::default()
        },
        &|| false,
    )
    .map_err(|error| error.describe())
}

fn owned_id(spec: &AnalyzerSpec, name: &str, owner: &str) -> Result<String, String> {
    let inspection = daemon(spec, &["container".into(), "inspect".into(), name.into()])?;
    if !inspection.succeeded() || inspection.truncated {
        return Err("container_inspect_unavailable".into());
    }
    let data: Value =
        serde_json::from_slice(&inspection.stdout).map_err(|_| "container_inspect_invalid")?;
    let rows = data
        .as_array()
        .filter(|rows| rows.len() == 1)
        .ok_or("container_inspect_ambiguous")?;
    let row = &rows[0];
    let id = row["Id"]
        .as_str()
        .filter(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("container_id_invalid")?;
    if row["Name"].as_str() != Some(format!("/{name}").as_str())
        || row["Config"]["Labels"][OWNER_LABEL].as_str() != Some(owner)
    {
        return Err("container_owner_mismatch".into());
    }
    Ok(id.to_string())
}

fn absent(spec: &AnalyzerSpec, name: &str) -> Result<bool, String> {
    let result = daemon(
        spec,
        &[
            "container".into(),
            "ls".into(),
            "--all".into(),
            "--no-trunc".into(),
            "--filter".into(),
            format!("name=^/{name}$"),
            "--format".into(),
            "{{.ID}}".into(),
        ],
    )?;
    if !result.succeeded() || result.truncated {
        return Err("container_absence_unconfirmed".into());
    }
    Ok(result.stdout_text().trim().is_empty())
}

fn cleanup(spec: &AnalyzerSpec, name: &str, owner: &str) -> Result<(), String> {
    match owned_id(spec, name, owner) {
        Ok(id) => {
            // Do not remove by a broad name/prefix, prune, or trust a caller's ID.
            // This exact ID came from inspecting our unique name and owner label.
            let removed = daemon(
                spec,
                &["container".into(), "rm".into(), "--force".into(), id],
            )?;
            if !removed.succeeded() {
                return Err("container_remove_failed".into());
            }
        }
        Err(error) if error == "container_owner_mismatch" || error == "container_id_invalid" => {
            return Err(error)
        }
        Err(_) => {} // Only a successful exact-name inventory can prove absence.
    }
    if !absent(spec, name)? {
        return Err("container_still_present".into());
    }
    Ok(())
}

fn persist_cleanup(
    connection: &Connection,
    receipt: &str,
    result: &Result<(), String>,
) -> Result<(), String> {
    connection.execute(
        "UPDATE analyzer_container_receipts SET cleanup_status=?2,detail=?3,updated_at=datetime('now','localtime') WHERE receipt_id=?1",
        params![receipt, if result.is_ok() { "confirmed" } else { "unconfirmed" },
            result.as_ref().err().map(String::as_str).unwrap_or("owned_container_absent")],
    ).map_err(|error| format!("analyzer_cleanup_receipt_write:{error}"))?;
    Ok(())
}

pub(super) fn run(
    connection: &Connection,
    spec: &AnalyzerSpec,
    key: &str,
    purpose: &str,
    planned: &[String],
    limits: &ProcessLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<BoundedRun, String> {
    run_with_logs(
        connection,
        spec,
        key,
        purpose,
        planned,
        limits,
        RunControl {
            cancelled,
            logged: false,
        },
    )
}

pub(super) struct RunControl<'a> {
    pub cancelled: &'a dyn Fn() -> bool,
    pub logged: bool,
}

pub(super) fn run_with_logs(
    connection: &Connection,
    spec: &AnalyzerSpec,
    key: &str,
    purpose: &str,
    planned: &[String],
    limits: &ProcessLimits,
    control: RunControl<'_>,
) -> Result<BoundedRun, String> {
    let RunControl { cancelled, logged } = control;
    ensure_no_pending(connection, &spec.scan_id)?;
    let cancelled_now = cancelled();
    if cancelled_now || limits.timeout.is_zero() {
        return Ok(BoundedRun {
            exit: None,
            stdout: vec![],
            stderr: vec![],
            truncated: false,
            timed_out: !cancelled_now,
            cancelled: cancelled_now,
            duration: Duration::ZERO,
            cleanup_error: None,
        });
    }
    if planned.first().map(String::as_str) != Some("run") {
        return Err("invalid_container_plan".into());
    }
    let receipt = Uuid::new_v4().simple().to_string();
    let name = format!("oviraptor-analyzer-{receipt}");
    let owner = Uuid::new_v4().simple().to_string();
    let mut create = vec![
        "create".into(),
        "--name".into(),
        name.clone(),
        "--label".into(),
        format!("{OWNER_LABEL}={owner}"),
    ];
    create.extend(
        planned
            .iter()
            .skip(1)
            .filter(|arg| arg.as_str() != "--rm")
            .cloned(),
    );
    connection.execute(
        "INSERT INTO analyzer_container_receipts(receipt_id,invocation_key,scan_id,attempt_number,purpose,container_name,owner_token,create_args_json)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![receipt, key, spec.scan_id, spec.attempt_number, purpose, name, owner, serde_json::to_string(&create).map_err(|e| e.to_string())?],
    ).map_err(|error| format!("analyzer_cleanup_receipt_create:{error}"))?;

    let mut uncertain_create = false;
    let mut never_spawned = false;
    let execution = (|| {
        let creation = match process::run(
            &spec.program,
            &create,
            Some(&spec.scratch_dir),
            &ProcessLimits {
                timeout: limits.timeout.min(Duration::from_secs(30)),
                stdout_bytes: 16 * 1024,
                stderr_bytes: 16 * 1024,
                ..limits.clone()
            },
            cancelled,
        ) {
            Ok(result) => result,
            Err(error) => {
                // Only Missing proves that spawn did not happen. Launch also
                // covers pipe setup/wait errors AFTER spawn, so treat its daemon
                // request as uncertain and attempt owned cleanup.
                never_spawned = error.is_missing();
                uncertain_create = !never_spawned;
                return Err(error.describe());
            }
        };
        if !creation.succeeded() {
            uncertain_create =
                creation.timed_out || creation.cancelled || creation.cleanup_error.is_some();
            return Ok(creation);
        }
        let id = owned_id(spec, &name, &owner)?;
        if creation.stdout_text().trim() != id {
            return Err("container_create_id_mismatch".into());
        }
        connection
            .execute(
                "UPDATE analyzer_container_receipts SET container_id=?2 WHERE receipt_id=?1",
                params![receipt, id],
            )
            .map_err(|error| error.to_string())?;
        // Cancel is checked again inside the bounded runner BEFORE daemon start.
        let args = ["start".into(), "--attach".into(), id];
        let run = if logged {
            super::process::log::driver::run(
                connection, spec, key, purpose, &args, limits, cancelled,
            )
        } else {
            process::run(
                &spec.program,
                &args,
                Some(&spec.scratch_dir),
                limits,
                cancelled,
            )
        };
        run.map_err(|error| error.describe())
    })();
    // This runs even if ownership verification or storing the ID failed. If create
    // was interrupted, absence now cannot prove an in-flight daemon request ended.
    let mut cleaned = if never_spawned {
        Ok(())
    } else {
        cleanup(spec, &name, &owner)
    };
    if uncertain_create {
        cleaned = Err("container_create_outcome_uncertain".into());
    }
    persist_cleanup(connection, &receipt, &cleaned)?;
    cleaned.map_err(|error| format!("analyzer_cleanup_unconfirmed:{receipt}:{error}"))?;
    execution
}
