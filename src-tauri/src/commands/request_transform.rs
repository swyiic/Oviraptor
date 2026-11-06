// D6 Stage 1 — programmable per-request transform MECHANISM (设计稿 §4e). The agent
// reverse-engineers a client crypto / signing / anti-replay scheme offline, then
// registers a pair of sandboxed Python transforms for the scan:
//   encode(req_bytes)  -> transformed_bytes   (fresh one-time key/IV + signature)
//   decode(resp_bytes) -> plaintext_bytes
// Transforms run under the SAME no-network Seatbelt sandbox as run_analysis_script
// (network denied, FS-isolated, time/mem-capped) — they only reshape BYTES; they
// never choose a target or reach the network.
//
// THIS STAGE builds the registry + the sandbox runner + the apply hooks only. It does
// NOT splice into the live broker send/response path — that is Stage 2, gated behind
// OVIRAPTOR_REQUEST_TRANSFORM (operator-only env; a target page cannot set it),
// DEFAULT OFF, so today's egress path in exchange.rs stays byte-identical. The apply
// hooks here are therefore no-ops until Stage 2 wires them and the flag is set.
//
// Floor (Stage 2): the transform only reshapes bytes going to an ALREADY-authorized
// target; the URL / scope is still decided by the SSRF/scope hard guard and the
// broker, never by the transform. A transform cannot smuggle a request to another host.

const REQUEST_TRANSFORM_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS request_transforms (
    scan_id TEXT PRIMARY KEY,
    attempt_number INTEGER NOT NULL,
    encode_script TEXT NOT NULL DEFAULT '',
    decode_script TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
"#;

const TRANSFORM_MAX_SCRIPT_BYTES: usize = 64 * 1024;
const TRANSFORM_MAX_IO_BYTES: usize = 1024 * 1024;

/// Stage-2 live splice is gated behind this env (operator-only; a target page cannot
/// set env). Default-absent = OFF = the broker path is untouched and the apply hooks
/// below are no-ops.
fn request_transform_enabled() -> bool {
    std::env::var("OVIRAPTOR_REQUEST_TRANSFORM").ok().as_deref() == Some("1")
}

#[derive(Default, Debug, PartialEq)]
struct RegisteredTransform {
    encode: String,
    decode: String,
}

fn read_request_transform(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Option<RegisteredTransform> {
    connection
        .query_row(
            "SELECT encode_script, decode_script FROM request_transforms WHERE scan_id=?1",
            [scan_id],
            |r| {
                Ok(RegisteredTransform {
                    encode: r.get::<_, String>(0)?,
                    decode: r.get::<_, String>(1)?,
                })
            },
        )
        .ok()
}

/// Run one transform script over `input` in the no-network sandbox and return its
/// stdout bytes. Bounds are checked BEFORE spawning. Never reaches the network.
fn run_request_transform(
    script: &str,
    input: &[u8],
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, String> {
    if script.trim().is_empty() {
        return Err("transform_script_empty".into());
    }
    if script.len() > TRANSFORM_MAX_SCRIPT_BYTES {
        return Err("transform_script_too_large".into());
    }
    if input.len() > TRANSFORM_MAX_IO_BYTES {
        return Err("transform_input_too_large".into());
    }
    let dir = std::env::temp_dir().join(format!("oviraptor-xform-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|_| "transform_scratch_unavailable".to_string())?;
    let written = std::fs::write(dir.join("xform.py"), script.as_bytes())
        .and_then(|_| std::fs::write(dir.join("input.dat"), input));
    if written.is_err() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err("transform_scratch_unavailable".into());
    }
    let run = crate::tool_supply::macos_static_parser::run_sandboxed(
        std::path::Path::new("/usr/bin/python3"),
        &["xform.py".to_string()],
        &dir,
        &dir,
        &crate::tool_supply::macos_static_parser::SandboxLimits::default(),
        cancelled,
    );
    let _ = std::fs::remove_dir_all(&dir);
    match run {
        Ok(r) if r.exit_code == Some(0) && !r.timed_out => {
            if r.stdout.len() > TRANSFORM_MAX_IO_BYTES {
                return Err("transform_output_too_large".into());
            }
            Ok(r.stdout)
        }
        Ok(r) => Err(format!(
            "transform_failed:exit={:?}:timeout={}",
            r.exit_code, r.timed_out
        )),
        Err(error) => Err(format!("transform_sandbox:{}", error.describe())),
    }
}

/// Stage-2 hook: encode an outgoing request body. No-op (None) unless the live splice
/// is enabled AND an encode transform is registered for the scan. Wired into the
/// broker in Stage 2; harmless until then.
#[allow(dead_code)]
fn apply_request_encode(
    connection: &rusqlite::Connection,
    scan_id: &str,
    body: &[u8],
) -> Option<Vec<u8>> {
    if !request_transform_enabled() {
        return None;
    }
    let transform = read_request_transform(connection, scan_id)?;
    if transform.encode.trim().is_empty() {
        return None;
    }
    run_request_transform(&transform.encode, body, &|| false).ok()
}

/// Stage-2 hook: decode an incoming response body. No-op (None) unless enabled AND a
/// decode transform is registered. Wired into the broker in Stage 2.
#[allow(dead_code)]
fn apply_response_decode(
    connection: &rusqlite::Connection,
    scan_id: &str,
    body: &[u8],
) -> Option<Vec<u8>> {
    if !request_transform_enabled() {
        return None;
    }
    let transform = read_request_transform(connection, scan_id)?;
    if transform.decode.trim().is_empty() {
        return None;
    }
    run_request_transform(&transform.decode, body, &|| false).ok()
}

/// Operator/agent: register the per-request encode/decode transforms for a scan.
/// Storing them is inert until Stage 2 splices the broker AND the flag is set.
#[tauri::command]
pub fn set_request_transform(
    state: State<AppState>,
    scan_id: String,
    encode_script: Option<String>,
    decode_script: Option<String>,
) -> Result<JsonValue, String> {
    if scan_id.trim().is_empty() || scan_id.contains('/') || scan_id.contains("..") {
        return Err("任务 ID 非法".into());
    }
    let encode = encode_script.unwrap_or_default();
    let decode = decode_script.unwrap_or_default();
    if encode.len() > TRANSFORM_MAX_SCRIPT_BYTES || decode.len() > TRANSFORM_MAX_SCRIPT_BYTES {
        return Err("transform 脚本过大".into());
    }
    let connection = db::open(&state.db_path)?;
    let exists: bool = connection
        .query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)", [&scan_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("任务不存在".into());
    }
    let attempt = scan_attempt_number(&connection, &scan_id);
    connection
        .execute_batch(REQUEST_TRANSFORM_SCHEMA)
        .map_err(|_| "request_transform_storage_unavailable")?;
    connection
        .execute(
            "INSERT INTO request_transforms(scan_id,attempt_number,encode_script,decode_script) VALUES(?1,?2,?3,?4) \
             ON CONFLICT(scan_id) DO UPDATE SET attempt_number=excluded.attempt_number,encode_script=excluded.encode_script,decode_script=excluded.decode_script,created_at=datetime('now','localtime')",
            params![scan_id, attempt, encode, decode],
        )
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "scanId": scan_id,
        "attemptNumber": attempt,
        "hasEncode": !encode.trim().is_empty(),
        "hasDecode": !decode.trim().is_empty(),
        "liveSpliceEnabled": request_transform_enabled(),
        "note": "变换机制已注册并可在无网沙箱运行;接入 broker 送发/响应路径为 Stage 2,受 OVIRAPTOR_REQUEST_TRANSFORM 旗控、默认关。未开旗时 egress 路径字节不变。"
    }))
}

#[cfg(test)]
mod request_transform_tests {
    use super::*;
    use rusqlite::Connection;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY, attempt_count INTEGER NOT NULL DEFAULT 1);
             INSERT INTO sentinel_scans(id,attempt_count) VALUES('s',1);",
        )
        .unwrap();
        c.execute_batch(REQUEST_TRANSFORM_SCHEMA).unwrap();
        c
    }

    #[test]
    fn registry_roundtrip_and_absent_is_none() {
        let c = db();
        assert!(read_request_transform(&c, "s").is_none());
        c.execute(
            "INSERT INTO request_transforms(scan_id,attempt_number,encode_script,decode_script) VALUES('s',1,'print(1)','print(2)')",
            [],
        )
        .unwrap();
        let t = read_request_transform(&c, "s").unwrap();
        assert_eq!(t.encode, "print(1)");
        assert_eq!(t.decode, "print(2)");
        assert!(read_request_transform(&c, "other").is_none());
    }

    #[test]
    fn apply_hooks_are_noop_while_live_splice_disabled() {
        // Default (env unset) → Stage-2 splice OFF → hooks never transform, even with
        // scripts registered. This guarantees the broker path stays byte-identical
        // until Stage 2 is explicitly enabled.
        let c = db();
        c.execute(
            "INSERT INTO request_transforms(scan_id,attempt_number,encode_script,decode_script) VALUES('s',1,'x','y')",
            [],
        )
        .unwrap();
        assert!(!request_transform_enabled());
        assert!(apply_request_encode(&c, "s", b"body").is_none());
        assert!(apply_response_decode(&c, "s", b"body").is_none());
    }

    #[test]
    fn run_transform_bounds_checked_before_spawn() {
        // These reject BEFORE launching the sandbox, so they are deterministic and
        // portable (no python / Seatbelt needed).
        assert!(run_request_transform("   ", b"x", &|| false).is_err());
        let huge_script = "#".repeat(TRANSFORM_MAX_SCRIPT_BYTES + 1);
        assert!(run_request_transform(&huge_script, b"x", &|| false).is_err());
        let huge_input = vec![0u8; TRANSFORM_MAX_IO_BYTES + 1];
        assert!(run_request_transform("print(1)", &huge_input, &|| false).is_err());
    }
}
