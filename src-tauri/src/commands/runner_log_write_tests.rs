use super::*;

struct RunnerLogWriteFixture {
    root: std::path::PathBuf,
    log: std::path::PathBuf,
}

impl RunnerLogWriteFixture {
    fn new() -> Self {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must be after the epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "oviraptor-runner-log-write-{}-{timestamp}-{serial}",
            std::process::id()
        ));
        let log = root.join("scan-fixture/attempt-1/runner.log");
        fs::create_dir_all(log.parent().expect("fixture log parent")).expect("create fixture");
        Self { root, log }
    }

    fn append_and_read_disk(&self, message: &str) -> String {
        append_runner_log(&self.log, message);
        // Read the persisted bytes directly: a redacted UI tail/event would
        // hide a write-time leak and cannot satisfy this assertion.
        fs::read_to_string(&self.log).expect("production append must create the runner log")
    }
}

impl Drop for RunnerLogWriteFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assert_runner_secret_absent(message: &str, secret: &str, context: &str) {
    let fixture = RunnerLogWriteFixture::new();
    let persisted = fixture.append_and_read_disk(message);
    assert!(
        persisted.contains(context),
        "ordinary diagnostic context must survive"
    );
    assert!(
        !persisted.contains(secret),
        "runner credentials must be masked before writing to disk"
    );
    assert!(
        persisted.contains("<redacted:"),
        "keep an auditable redaction marker"
    );
}

#[test]
fn append_runner_log_redacts_password_before_disk_write() {
    assert_runner_secret_absent(
        "frontend auth failed password=runner-password-synthetic retry=2",
        "runner-password-synthetic",
        "frontend auth failed",
    );
}

#[test]
fn append_runner_log_redacts_api_key_before_disk_write() {
    assert_runner_secret_absent(
        "model request failed api_key=runner-api-key-synthetic status=401",
        "runner-api-key-synthetic",
        "model request failed",
    );
}

#[test]
fn append_runner_log_redacts_bearer_before_disk_write() {
    assert_runner_secret_absent(
        "request failed\nAuthorization: Bearer runner-bearer-synthetic\nstatus=403",
        "runner-bearer-synthetic",
        "request failed",
    );
}

#[test]
fn append_runner_log_keeps_complete_normal_text_and_multiline_append() {
    let fixture = RunnerLogWriteFixture::new();
    let message = format!(
        "frontend retry=2 status=502；模型恢复就绪\n    next route=/catalog HTTP 200 {}",
        "正常 diagnostic 段落 ".repeat(200)
    );
    let first = fixture.append_and_read_disk(&message);
    assert!(
        message.chars().count() > 1_200,
        "exercise the UI display bound"
    );
    assert!(
        first.ends_with(&format!(" [oviraptor] {message}\n")),
        "persisted diagnostics must retain full normal text and line breaks"
    );
    let second = fixture.append_and_read_disk("runner completed 正常结束");
    assert!(
        second.starts_with(&first),
        "appending must not rewrite existing log bytes"
    );
    assert!(second.ends_with(" [oviraptor] runner completed 正常结束\n"));
}

#[test]
fn append_runner_log_redaction_preserves_native_json_stdout_contract() {
    let json_stdout = br#"{"schemaVersion":1,"routes":[{"path":"/catalog","password":"native-json-synthetic"}],"status":"completed"}"#;
    let (output, truncated) =
        drain_native_output(std::io::Cursor::new(json_stdout), 32 * 1024 * 1024);
    assert!(!truncated);
    assert_eq!(
        output.as_slice(),
        json_stdout,
        "Native JSON stdout remains byte-exact"
    );
    let parsed: JsonValue =
        serde_json::from_slice(&output).expect("Native stdout must remain JSON");
    assert_eq!(parsed["routes"][0]["password"], "native-json-synthetic");

    let fixture = RunnerLogWriteFixture::new();
    let persisted = fixture.append_and_read_disk(
        "Native JSON stdout accepted; helper stderr password=runner-stderr-synthetic",
    );
    assert!(persisted.contains("Native JSON stdout accepted"));
    assert!(!persisted.contains("runner-stderr-synthetic"));
    // Only the diagnostic append is filtered; no output/evidence file is
    // rewritten and the helper's machine-readable result stays untouched.
    assert_eq!(output.as_slice(), json_stdout);
    assert_eq!(parsed["schemaVersion"], 1);
}
