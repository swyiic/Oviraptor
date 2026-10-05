#[cfg(test)]
mod native_src_transport_tests {
    use super::*;
    #[test]
    fn native_race_respects_configured_limits_and_preserves_summary_contract() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for _ in 0..4 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = [0; 4096];
                let _ = socket.read(&mut request);
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .unwrap();
            }
        });
        let result = native_race_schedule(
            &serde_json::json!({"url":format!("http://{address}/fixture"),"method":"GET","attempts":4,"concurrency":2}),
            "127.0.0.1",
            &std::sync::atomic::AtomicBool::new(false),
        );
        server.join().unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["attempts"], 4);
        assert_eq!(result["concurrency"], 2);
        assert_eq!(result["statuses"]["200"], 4);
        assert_eq!(result["errors"], 0);
        assert_eq!(result["distinctResponseHashes"], 1);
    }
    #[test]
    fn native_raw_preserves_binary_body_duplicate_headers_and_response_bytes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut request = format!("POST /fixture HTTP/1.1\r\nHost: {address}\r\nX-Fixture: one\r\nX-Fixture: two\r\nContent-Length: 3\r\nConnection: close\r\n\r\n").into_bytes();
        request.extend_from_slice(&[0, 255, 1]);
        let expected = request.clone();
        let response =
            b"HTTP/1.1 200 OK\r\nX-Fixture: one\r\nX-Fixture: two\r\nContent-Length: 2\r\n\r\nok";
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = vec![0; expected.len()];
            socket.read_exact(&mut bytes).unwrap();
            assert_eq!(bytes, expected);
            socket.write_all(response).unwrap();
        });
        let result = native_raw_http(&format!("http://{address}"), &request);
        server.join().unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["responseBytes"], response.len());
        assert_eq!(
            result["responseSha256"],
            format!("{:x}", Sha256::digest(response))
        );
        assert_eq!(result["statusLine"], "HTTP/1.1 200 OK");
    }

    #[test]
    fn native_adapter_rejects_invalid_contracts_before_sending() {
        let target = reqwest::Url::parse("http://example.test:8080").unwrap();
        assert!(
            native_raw_request_method(&target, b"GET / HTTP/1.1\r\nHost: other.test\r\n\r\n")
                .is_err()
        );
        assert!(native_raw_request_method(&target, b"DELETE / HTTP/1.1\r\n\r\n").is_err());
        let contract = serde_json::json!({"url":"http://example.test/fixture","method":"POST"});
        assert_eq!(
            native_race_schedule(
                &contract,
                "example.test",
                &std::sync::atomic::AtomicBool::new(false)
            )["ok"],
            false
        );
        let oversized =
            serde_json::json!({"url":"http://example.test/fixture","body":"x".repeat(65537)});
        assert!(native_race_contract(&oversized, "example.test").is_err());
        let cancelled = native_race_schedule(
            &serde_json::json!({"url":"http://example.test/fixture","method":"GET"}),
            "example.test",
            &std::sync::atomic::AtomicBool::new(true),
        );
        assert_eq!(cancelled["ok"], false);
        assert_eq!(cancelled["cancelled"], true);
        assert_eq!(cancelled["errors"], 2);
    }
}
