use super::*;
use std::net::TcpListener;

fn fixture() -> (PathBuf, Context) {
    let root =
        std::env::temp_dir().join(format!("ovi-hunter-diagnostics-{}", uuid::Uuid::new_v4()));
    let context = Context::new(
        &root.join("test.sqlite3"),
        "direct".into(),
        Some(7),
        Some("query-test".into()),
    );
    (root, context)
}
fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}
fn server(status: &str, body: &str, headers: &str) -> (reqwest::Url, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    );
    let worker = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut b = [0; 4096];
        let _ = s.read(&mut b);
        s.write_all(response.as_bytes()).unwrap();
    });
    (
        reqwest::Url::parse(&format!("http://{address}/openApi/search")).unwrap(),
        worker,
    )
}
fn entries(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join("diagnostics/hunter.jsonl"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

#[test]
fn forbidden_response_keeps_status_and_safe_business_error_without_credentials_or_personal_info() {
    let (root, context) = fixture();
    let key = "synthetic-private-key";
    let body=json!({"code":403,"message":format!("denied key={key}, email=user@example.com phone=13900000000"),"data":{"personal_info":{"username":"private-name","phone":"13900000000"}}}).to_string();
    let(mut url,worker)=server("403 Forbidden",&body,"Content-Type: application/json\r\nX-Request-Id: provider-id\r\nSet-Cookie: private-cookie\r\n");
    url.query_pairs_mut()
        .append_pair("api-key", key)
        .append_pair(
            "search",
            &base64::engine::general_purpose::URL_SAFE.encode("cert.sha-256=\"abc\""),
        )
        .append_pair("page", "2")
        .append_pair("start_time", "2026-09-22");
    let error = get_json(&client(), url, key, &context).unwrap_err();
    worker.join().unwrap();
    let log = fs::read_to_string(root.join("diagnostics/hunter.jsonl")).unwrap();
    for secret in [
        key,
        "user@example.com",
        "13900000000",
        "private-name",
        "private-cookie",
        "api-key=",
    ] {
        assert!(!log.contains(secret), "leaked {secret}");
        assert!(!error.contains(secret));
    }
    let e = entries(&root);
    assert_eq!(e.len(), 2);
    assert_eq!(e[1]["details"]["httpStatus"], 403);
    assert_eq!(e[1]["details"]["provider"]["code"], 403);
    assert_eq!(e[0]["details"]["page"], "2");
    assert_eq!(e[1]["automaticRetries"], 0);
    assert!(error.contains("诊断编号"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            root.join("diagnostics/hunter.jsonl")
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn html_and_connection_errors_preserve_diagnostics_without_raw_pages_or_secret_urls() {
    let (root, context) = fixture();
    let (mut url, worker) = server(
        "200 OK",
        "<html>cookie=hidden-private-value</html>",
        "Content-Type: text/html\r\n",
    );
    url.query_pairs_mut().append_pair("api-key", "secret-token");
    assert!(get_json(&client(), url, "secret-token", &context)
        .unwrap_err()
        .contains("有效 JSON"));
    worker.join().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let url = reqwest::Url::parse(&format!(
        "http://127.0.0.1:{port}/openApi/userInfo?api-key=secret-token"
    ))
    .unwrap();
    assert!(get_json(&client(), url, "secret-token", &context).is_err());
    let e = entries(&root);
    assert_eq!(e.len(), 4);
    assert_eq!(e[1]["details"]["nonJsonBody"], true);
    assert_eq!(
        e[3]["details"]["transportCategory"],
        "connect_dns_tls_or_proxy"
    );
    assert!(e[3]["details"]["errorChain"].as_array().unwrap().len() > 1);
    let log = fs::read_to_string(root.join("diagnostics/hunter.jsonl")).unwrap();
    assert!(!log.contains("secret-token"));
    assert!(!log.contains("hidden-private-value"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quota_response_and_rotation_keep_structured_json_and_limit_history() {
    let (root, context) = fixture();
    let body=json!({"code":200,"data":{"total":0,"arr":[],"consume_quota":"消耗积分：0","rest_quota":"剩余积分：9"}}).to_string();
    let (url, worker) = server("200 OK", &body, "Content-Type: application/json\r\n");
    let value = get_json(&client(), url, "", &context).unwrap();
    worker.join().unwrap();
    assert_eq!(value["data"]["total"], 0);
    assert_eq!(
        entries(&root)[1]["details"]["provider"]["consumeQuota"],
        "消耗积分：0"
    );
    let file = root.join("diagnostics/hunter.jsonl");
    OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_len(LIMIT)
        .unwrap();
    context.event("", "test_rotation", json!({})).unwrap();
    assert!(file.with_extension("previous.jsonl").exists());
    assert_eq!(entries(&root).len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_unwritable_diagnostic_destination_prevents_network() {
    let (root, context) = fixture();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("diagnostics"), "blocked directory").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = reqwest::Url::parse(&format!(
        "http://{}/openApi/search",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    assert!(get_json(&client(), url, "key", &context)
        .unwrap_err()
        .contains("诊断目录"));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "explicit opt-in; reads only the saved key and calls userInfo, never search"]
fn live_saved_account_connectivity_probe() {
    let database =
        std::env::var_os("OVI_HUNTER_PROBE_DATABASE").expect("explicit database required");
    let output =
        std::env::var_os("OVI_HUNTER_PROBE_OUTPUT").expect("isolated diagnostic output required");
    let c = Connection::open_with_flags(
        PathBuf::from(database),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (_, key, _, _) = setting(&c).unwrap();
    assert!(!key.is_empty(), "no saved Hunter key");
    let settings = crate::model_configuration::default_settings_for(&c, &["network"]);
    assert!(settings.get("_configurationError").is_none());
    let mode = super::super::hunter_network::mode(&c).unwrap();
    let context = Context::new(
        &PathBuf::from(output).join("probe.sqlite3"),
        mode.clone(),
        None,
        None,
    );
    let client = super::super::hunter_network::client(&c, &settings, 20).unwrap();
    match hunter_user_info(&client, &key, &context) {
        Ok(account) => {
            println!("Account connectivity: success; mode={mode}; safe quota summary={account}")
        }
        Err(error) => println!("Account connectivity: failed; mode={mode}; {error}"),
    }
}
