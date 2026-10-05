// Actual localhost size-boundary contracts, separate from worker authorization.
#[cfg(test)]
mod native_race_boundary_tests {
    use super::*;

    // Real target I/O, bounded teardown even when the expected rejection is red.
    fn observed_boundary_call(mut contract: JsonValue) -> (JsonValue, usize, usize) {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        contract["url"] = serde_json::json!(format!("http://{address}/fixture"));
        let stop = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let control = Arc::clone(&stop);
        let calls = Arc::clone(&count);
        let live = Arc::clone(&active);
        let maximum = Arc::clone(&peak);
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut handlers = Vec::new();
            while !control.load(Ordering::Acquire) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let calls = Arc::clone(&calls);
                        let live = Arc::clone(&live);
                        let maximum = Arc::clone(&maximum);
                        handlers.push(std::thread::spawn(move || {
                            stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
                            stream.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
                            let mut bytes = [0; 4096];
                            let size = stream.read(&mut bytes).unwrap_or(0);
                            if size == 0 { return; }
                            calls.fetch_add(1, Ordering::AcqRel);
                            let current = live.fetch_add(1, Ordering::AcqRel) + 1;
                            maximum.fetch_max(current, Ordering::AcqRel);
                            std::thread::sleep(Duration::from_millis(30));
                            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
                            live.fetch_sub(1, Ordering::AcqRel);
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("localhost boundary fixture: {error}"),
                }
            }
            for handler in handlers {
                handler.join().unwrap();
            }
        });
        let result = native_race_schedule(&contract, "127.0.0.1", &AtomicBool::new(false));
        stop.store(true, Ordering::Release);
        server.join().unwrap();
        (
            result,
            count.load(Ordering::Acquire),
            peak.load(Ordering::Acquire),
        )
    }

    #[test]
    fn native_race_above_three_is_rejected_before_target_io() {
        let (result, calls, peak) = observed_boundary_call(serde_json::json!({
            "method":"GET", "concurrency":4, "attempts":4
        }));
        assert_eq!(
            calls, 0,
            "unapproved four-way contract reached target; peak={peak}; result={result}"
        );
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"], "race_concurrency_invalid");
    }

    #[test]
    fn native_race_default_is_two_requests_and_two_workers() {
        let (result, calls, peak) = observed_boundary_call(serde_json::json!({"method":"GET"}));
        assert_eq!(
            calls, 2,
            "default must not silently repeat a pressure batch; peak={peak}; result={result}"
        );
        assert_eq!(result["ok"], true);
        assert_eq!(result["concurrency"], 2);
        assert_eq!(result["attempts"], 2);
    }

    #[test]
    fn native_race_malformed_limits_do_not_become_executable_defaults() {
        for value in [
            serde_json::json!(null),
            serde_json::json!("3"),
            serde_json::json!(2.5),
            serde_json::json!(0),
            serde_json::json!(1),
        ] {
            let (result, calls, _) = observed_boundary_call(serde_json::json!({
                "method":"GET", "concurrency":value, "attempts":2
            }));
            assert_eq!(
                calls, 0,
                "malformed concurrency performed target I/O: {result}"
            );
            assert_eq!(result["ok"], false);
            assert_eq!(result["error"], "race_concurrency_invalid");
        }
        for value in [
            serde_json::json!(null),
            serde_json::json!("2"),
            serde_json::json!(2.5),
            serde_json::json!(0),
            serde_json::json!(1),
            serde_json::json!(129),
        ] {
            let (result, calls, _) = observed_boundary_call(serde_json::json!({
                "method":"GET", "concurrency":2, "attempts":value
            }));
            assert_eq!(
                calls, 0,
                "malformed attempts performed target I/O: {result}"
            );
            assert_eq!(result["ok"], false);
            assert_eq!(result["error"], "race_attempts_invalid");
        }
    }
}
