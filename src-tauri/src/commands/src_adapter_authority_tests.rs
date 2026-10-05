#[cfg(test)]
mod src_adapter_authority_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    // Exercise the actual HTTP receiver, not the standalone transport fixture.
    fn observe_unbound_route(kind: &str, write: bool) -> (u16, JsonValue, usize, u16) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicUsize::new(0));
        let stopping = Arc::clone(&stop);
        let calls = Arc::clone(&count);
        let target = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !stopping.load(Ordering::Acquire) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        socket
                            .set_write_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut bytes = [0; 8192];
                        if socket.read(&mut bytes).unwrap_or(0) > 0 {
                            calls.fetch_add(1, Ordering::AcqRel);
                            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("local target: {error}"),
                }
            }
        });
        let root =
            std::env::temp_dir().join(format!("oviraptor-adapter-no-worker-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let receiver = stage_builtin_src_assurance(&format!("http://{address}"), &root).unwrap();
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap();
        let body = if kind == "raw" {
            format!("{} /unbound HTTP/1.1\r\nHost: {address}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", if write { "POST" } else { "GET" })
        } else {
            serde_json::json!({
                "url":format!("http://{address}/unbound"), "method":if write {"POST"} else {"GET"},
                "concurrency":2,"attempts":2,
                "cleanup":{"url":format!("http://{address}/cleanup"),"method":"POST"},
                "invariant":"caller supplied text is not a frozen business contract"
            })
            .to_string()
        };
        let url = if kind == "raw" {
            &receiver.raw_url
        } else {
            &receiver.race_url
        };
        let result = client.post(url).body(body).send().unwrap();
        let status = result.status().as_u16();
        let answer: JsonValue = serde_json::from_str(&result.text().unwrap()).unwrap();
        let callback = client
            .get(&receiver.base_url)
            .send()
            .unwrap()
            .status()
            .as_u16();
        drop(receiver);
        stop.store(true, Ordering::Release);
        target.join().unwrap();
        fs::remove_dir_all(root).unwrap();
        (status, answer, count.load(Ordering::Acquire), callback)
    }

    #[test]
    fn src_adapter_unbound_raw_and_race_never_send_target_requests() {
        let mut observations = Vec::new();
        for kind in ["raw", "race"] {
            for write in [false, true] {
                observations.push((kind, write, observe_unbound_route(kind, write)));
            }
        }
        for (kind, write, (status, answer, calls, callback)) in observations {
            assert_eq!(calls, 0, "{kind}, write={write}, result={answer}");
            assert_eq!(status, 403);
            assert_eq!(answer["ok"], false);
            assert_eq!(answer["error"], "adapter_worker_contract_required");
            assert_eq!(callback, 204, "passive OAST receiver remains usable");
        }
    }

    #[test]
    fn src_adapter_manifest_does_not_advertise_unbound_execution() {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-adapter-advertisement-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let receiver = stage_builtin_src_assurance("http://127.0.0.1", &root).unwrap();
        let manifest: JsonValue =
            serde_json::from_slice(&fs::read(root.join(SRC_ASSURANCE_ADAPTER_NAME)).unwrap())
                .unwrap();
        drop(receiver);
        fs::remove_dir_all(root).unwrap();
        assert_eq!(manifest["adapter"]["rawHttp"]["available"], false);
        assert_eq!(manifest["adapter"]["raceScheduler"]["available"], false);
        assert_eq!(manifest["adapter"]["commands"], serde_json::json!({}));
        let capabilities = web_capability_manifest(&serde_json::json!({}), 0);
        assert_eq!(capabilities["rawHttpProtocol"]["available"], false);
        assert_eq!(capabilities["raceScheduler"]["available"], false);
        assert_eq!(capabilities["controlledWrite"]["available"], false);
    }
}
