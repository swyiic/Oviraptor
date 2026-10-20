use super::*;
use std::{collections::HashMap, io::Write, net::TcpListener, thread};

fn local_account(
    payload: JsonValue,
    email: &str,
    status: u16,
) -> (Result<JsonValue, String>, reqwest::Url) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let size = socket.read(&mut buffer).unwrap();
            assert!(size > 0, "account request headers incomplete");
            bytes.extend_from_slice(&buffer[..size]);
            if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                break;
            }
            assert!(bytes.len() < 16 * 1024);
        }
        let message = String::from_utf8(bytes).unwrap();
        let target = message
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = reqwest::Url::parse(&format!("http://{address}{target}")).unwrap();
        let body = payload.to_string();
        write!(socket, "HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        url
    });
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let result = request_fofa_account_with_client(
        &client,
        &format!("http://{address}/api/v1/info/my"),
        "synthetic-test-key",
        email,
        "",
    );
    (result, worker.join().unwrap())
}

#[test]
fn optional_account_email_preserves_key_only_and_encodes_query_values() {
    let key_only =
        fofa_account_endpoint("https://fofa.info/api/v1/info/my", "fixture-key", "").unwrap();
    assert_eq!(
        key_only.query_pairs().collect::<Vec<_>>(),
        vec![("key".into(), "fixture-key".into())]
    );
    let email = "local+tag&key=another@example.test";
    let bound =
        fofa_account_endpoint("https://fofa.info/api/v1/info/my", "fixture-key", email).unwrap();
    let pairs = bound.query_pairs().collect::<HashMap<_, _>>();
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs["email"], email);
    assert_eq!(pairs["key"], "fixture-key");
    assert_eq!(
        normalize_fofa_email("  local+tag@example.test ").unwrap(),
        "local+tag@example.test"
    );
    for invalid in [
        "missing-at",
        "@example.test",
        "local@",
        "a@@b.test",
        "a b@example.test",
        "a\nb@example.test",
    ] {
        assert!(normalize_fofa_email(invalid).is_err());
    }
    assert!(normalize_fofa_email(&format!("{}@example.test", "a".repeat(321))).is_err());
}

#[test]
fn actual_account_request_sends_and_checks_selected_email() {
    let (result, url) = local_account(
        serde_json::json!({"error":false,"email":"Local+Tag@example.test","vip_level":"fixture"}),
        "local+tag@example.test",
        200,
    );
    assert!(result.is_ok());
    let pairs = url.query_pairs().collect::<HashMap<_, _>>();
    assert_eq!(pairs["email"], "local+tag@example.test");
    assert_eq!(pairs["key"], "synthetic-test-key");
}

#[test]
fn authenticated_masked_email_is_not_mistaken_for_an_account_mismatch() {
    for masked in ["********", "lo***@example.test"] {
        let (result, url) = local_account(
            serde_json::json!({"error":false,"email":masked,"vip_level":1}),
            "local@example.test",
            200,
        );
        assert!(result.is_ok(), "masked identity is not a different account: {result:?}");
        assert!(url.query_pairs().any(|(key, value)| key == "email" && value == "local@example.test"));
    }
}

#[test]
fn masked_identity_never_overrides_a_failed_or_unconfirmed_authentication() {
    for error in [serde_json::json!(true), serde_json::Value::Null, serde_json::json!("false")] {
        let (result, _) = local_account(
            serde_json::json!({"error":error,"email":"********"}),
            "local@example.test",
            200,
        );
        assert!(result.is_err());
    }
}

#[test]
fn actual_account_mismatch_and_missing_identity_are_not_success() {
    let (mismatch, _) = local_account(
        serde_json::json!({"error":false,"email":"different@example.test"}),
        "selected@example.test",
        200,
    );
    assert!(mismatch.unwrap_err().contains("不匹配"));
    let (missing, _) = local_account(
        serde_json::json!({"error":false,"username":"selected@example.test"}),
        "selected@example.test",
        200,
    );
    assert!(missing.unwrap_err().contains("未返回账户邮箱"));
    let (legacy, url) = local_account(
        serde_json::json!({"error":false,"username":"legacy"}),
        "",
        200,
    );
    assert!(legacy.is_ok());
    assert!(!url.query_pairs().any(|(key, _)| key == "email"));
}

#[test]
fn remote_authentication_errors_cannot_echo_keys_or_proxy_userinfo() {
    let sensitive = "synthetic-test-key http://proxy-user:proxy-password@127.0.0.1:1";
    let (failed, _) = local_account(
        serde_json::json!({"error":true,"errmsg":sensitive}),
        "",
        401,
    );
    let message = failed.unwrap_err();
    assert!(message.contains("401"));
    for secret in ["synthetic-test-key", "proxy-user", "proxy-password"] {
        assert!(!message.contains(secret));
    }
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(100))
        .build()
        .unwrap();
    let error = client.get("http://127.0.0.1:0").send().unwrap_err();
    let message = fofa_transport_error(&error, "http://proxy-user:proxy-password@127.0.0.1:1");
    assert!(message.contains("代理"));
    assert!(!message.contains("proxy-user"));
    assert!(!message.contains("proxy-password"));
}

#[test]
fn omitted_email_is_backward_compatible_in_test_command_input() {
    let legacy: FofaApiTestInput =
        serde_json::from_value(serde_json::json!({"key":"fixture-key"})).unwrap();
    assert_eq!(legacy.email, "");
    let current: FofaApiTestInput = serde_json::from_value(
        serde_json::json!({"key":"fixture-key","email":"selected@example.test"}),
    )
    .unwrap();
    assert_eq!(current.email, "selected@example.test");
}
