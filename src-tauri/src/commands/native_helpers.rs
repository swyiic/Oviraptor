/// One cancellation token for the complete native frontend attempt.
#[derive(Clone)]
struct NativeReconControl {
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    deadline: Instant,
}

impl NativeReconControl {
    fn new(timeout: Duration) -> Self {
        Self {
            cancelled: Default::default(),
            deadline: Instant::now() + timeout,
        }
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

fn drain_native_output(mut stream: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    let mut truncated = false;
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(size) => {
                let keep = size.min(limit.saturating_sub(output.len()));
                output.extend_from_slice(&buffer[..keep]);
                truncated |= keep < size;
            }
        }
    }
    (output, truncated)
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

fn run_native_json_helper(
    helper: &Path,
    payload: &JsonValue,
    runtime_path: &OsString,
    proxy: Option<&str>,
    no_proxy: &str,
    timeout: Duration,
    control: &NativeReconControl,
) -> Result<JsonValue, String> {
    control.check()?;
    let input = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    let mut command = Command::new("node");
    configure_child_command(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .arg(helper)
        .env("PATH", runtime_path)
        .env("NO_PROXY", no_proxy)
        .env("no_proxy", no_proxy)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
        command.env("HTTPS_PROXY", proxy).env("HTTP_PROXY", proxy);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 {}：{error}", helper.display()))?;
    // The three pipes must be drained concurrently; otherwise a large JSON
    // capture can fill stdout and deadlock a parent waiting for process exit.
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let writer = thread::spawn(move || stdin.write_all(&input));
    let stdout_reader = thread::spawn(move || drain_native_output(stdout, 32 * 1024 * 1024));
    let stderr_reader = thread::spawn(move || drain_native_output(stderr, 64 * 1024));
    let started = Instant::now();
    let status = loop {
        if let Err(error) = control.check() {
            break Err(error);
        }
        if started.elapsed() >= timeout {
            break Err(format!(
                "{} 超过 {} 秒",
                helper.display(),
                timeout.as_secs()
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(error) => break Err(error.to_string()),
            Ok(None) => thread::sleep(Duration::from_millis(100)),
        }
    };
    if status.is_err() {
        force_stop_sentinel_process(child.id() as i64);
        let _ = child.kill();
        let _ = child.wait();
    }
    let written = writer.join().map_err(|_| "helper 输入线程退出".to_string());
    let (output, truncated) = stdout_reader
        .join()
        .map_err(|_| "helper 输出线程退出".to_string())?;
    let (error_output, _) = stderr_reader
        .join()
        .map_err(|_| "helper 错误输出线程退出".to_string())?;
    let status = status?;
    written?.map_err(|error| error.to_string())?;
    control.check()?;
    if !status.success() {
        return Err(format!(
            "{} {status}：{}",
            helper.display(),
            String::from_utf8_lossy(&error_output)
                .chars()
                .take(1200)
                .collect::<String>()
        ));
    }
    if truncated {
        return Err("helper JSON 超过 32 MiB，未将截断数据当作完整证据".into());
    }
    serde_json::from_slice(&output)
        .map_err(|error| format!("{} JSON 无效：{error}", helper.display()))
}

#[cfg(test)]
mod native_helper_tests {
    use super::*;

    fn helper(
        source: &str,
        timeout: Duration,
        control: &NativeReconControl,
    ) -> Result<JsonValue, String> {
        let directory =
            std::env::temp_dir().join(format!("oviraptor-helper-test-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("fixture.cjs");
        fs::write(&path, source).unwrap();
        let result = run_native_json_helper(
            &path,
            &serde_json::json!({"fixture":true}),
            &std::env::var_os("PATH").unwrap_or_default(),
            None,
            "",
            timeout,
            control,
        );
        fs::remove_dir_all(&directory).unwrap();
        result
    }

    #[test]
    fn large_stdout_and_stderr_do_not_deadlock() {
        let control = NativeReconControl::new(Duration::from_secs(10));
        let value = helper("require('fs').readFileSync(0); process.stderr.write('w'.repeat(200000)); process.stdout.write(JSON.stringify({body:'x'.repeat(300000)}));",
            Duration::from_secs(5), &control).unwrap();
        assert_eq!(value["body"].as_str().unwrap().len(), 300000);
    }

    #[test]
    fn cancelled_helper_stops_promptly() {
        let control = NativeReconControl::new(Duration::from_secs(10));
        let cancellation = control.clone();
        let trigger = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            cancellation.cancel();
        });
        let started = Instant::now();
        let error = helper(
            "require('fs').readFileSync(0); setInterval(()=>{},1000);",
            Duration::from_secs(8),
            &control,
        )
        .unwrap_err();
        trigger.join().unwrap();
        assert!(error.contains("取消"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn malformed_json_is_not_accepted_as_success() {
        let control = NativeReconControl::new(Duration::from_secs(10));
        let error = helper(
            "require('fs').readFileSync(0); process.stdout.write('{broken');",
            Duration::from_secs(5),
            &control,
        )
        .unwrap_err();
        assert!(error.contains("JSON 无效"));
    }
}
