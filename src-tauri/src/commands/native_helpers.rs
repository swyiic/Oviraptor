/// One paused browser request: method, url, and the reply the probe waits on
/// before `Fetch.continueRequest`.
type BrowserBrokerAsk = mpsc::Sender<(String, String, mpsc::Sender<bool>)>;

/// One cancellation token for the complete native frontend attempt.
#[derive(Clone)]
struct NativeReconControl {
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    deadline: Instant,
    log_owner: Option<NativeHelperLogBinding>,
    browser_broker: Option<BrowserBrokerAsk>,
}

impl NativeReconControl {
    fn new(timeout: Duration) -> Self {
        Self {
            cancelled: Default::default(),
            deadline: Instant::now() + timeout,
            log_owner: None,
            browser_broker: None,
        }
    }

    fn new_logged(timeout:Duration,path:&Path,scan_id:&str,attempt:i64,target:&str)->Result<Self,String> {
        let mut control=Self::new(timeout);
        control.log_owner=Some(NativeHelperLogBinding::load(path,scan_id,attempt,target)?);
        Ok(control)
    }
    fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }

    fn check(&self) -> Result<(), String> {
        if self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            Err("前端侦察已取消；不会发布晚到结果".into())
        } else if Instant::now() >= self.deadline {
            Err("前端侦察已达到本轮时限；不会发布晚到结果".into())
        } else {
            Ok(())
        }
    }
}

fn read_native_http_body(
    mut response: reqwest::blocking::Response,
    limit: usize,
) -> Result<String, String> {
    let mut bytes = Vec::new();
    Read::by_ref(&mut response)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err(format!("响应体超过 {limit} 字节，未解析截断内容"));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// How the Node helper itself stopped. The exit code and signal are kept apart
/// because a kill-by-signal run has no code at all.
fn native_helper_exit_label(status: &std::process::ExitStatus) -> String {
    #[cfg(unix)]
    let signal = status.signal();
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    if let Some(signal) = signal {
        return format!("辅助程序被信号 {signal} 终止");
    }
    match status.code() {
        Some(code) => format!("辅助程序退出码 {code}"),
        None => "辅助程序退出状态未知".to_string(),
    }
}

/// Helper stderr is a local diagnostic, not evidence: it is capped and any
/// credential-shaped material is masked before it can be stored or displayed.
fn native_helper_stderr_note(error_output: &[u8]) -> String {
    let text = String::from_utf8_lossy(error_output);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "辅助程序未输出 stderr".to_string();
    }
    bounded_redacted_text(trimmed, 1200)
}

// Both old JSON API and diagnostics use this actual private capture path.
fn run_native_json_helper(helper:&Path,payload:&JsonValue,runtime_path:&OsString,
 proxy:Option<&str>,no_proxy:&str,timeout:Duration,control:&NativeReconControl)->Result<JsonValue,String>{
 let output=run_native_json_helper_raw(helper,payload,runtime_path,proxy,no_proxy,timeout,control)?;
 serde_json::from_slice(&output).map_err(|error|format!("{} JSON 无效：{error}",helper.display()))
}


