use super::*;

fn options() -> ProbeOptions {
    ProbeOptions {
        workers: 1,
        timeout: Duration::from_secs(3),
        total_timeout: Duration::from_secs(4),
        retries: 0,
        max_body_bytes: 4096,
        include_other: false,
        include_weak: false,
        allow_private: true,
        strict_tls: false,
        scheme_fallback: false,
        content_threshold: 12,
        replace_default_content_rules: false,
        gambling_keywords: vec![],
        porn_keywords: vec![],
        negative_keywords: vec![],
        custom_keywords: vec![],
        priority_rate: 1000.0,
        other_rate: 1000.0,
        per_host_interval: Duration::ZERO,
    }
}

fn serve_tls(body: &'static [u8]) -> (String, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(
                    include_bytes!("../../test-fixtures/tls/leaf.der").to_vec(),
                )],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(
                        include_bytes!("../../test-fixtures/tls/test-only-key.der").to_vec(),
                    ),
                ),
            )
            .unwrap();
        let (tcp, _) = listener.accept().unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut tls = rustls::StreamOwned::new(
            rustls::ServerConnection::new(Arc::new(config)).unwrap(),
            tcp,
        );
        let mut request = [0; 4096];
        if tls.read(&mut request).is_ok() {
            let _ = tls.write_all(body);
            let _ = tls.flush();
        }
    });
    (format!("https://{address}/"), handle)
}

#[test]
fn actual_https_probe_csv_preserves_the_leaf_der_and_openssl_fingerprint_even_for_restricted_http()
{
    let (url, server) = serve_tls(
        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 6\r\nConnection: close\r\n\r\ndenied",
    );
    let root = std::env::temp_dir().join(format!("ovi-tls-probe-{}", uuid::Uuid::new_v4()));
    let input = root.join("input");
    let output = root.join("output");
    fs::create_dir_all(&input).unwrap();
    let header = "company,asset_key,host,link,protocol\n";
    fs::write(
        input.join("P1_active_strong.csv"),
        format!("{header}Fixture,{url},{url},{url},https\n"),
    )
    .unwrap();
    for file in ["P2_strong_needs_validation.csv", "P3_name_candidates.csv"] {
        fs::write(input.join(file), header).unwrap();
    }
    let options = options();
    probe_assets(
        &input,
        &input.join("P1_active_strong.csv"),
        &output,
        options,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    server.join().unwrap();
    let (_, rows) = read_csv(&output.join("P1_web_restricted.csv")).unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["probe_cert_capture"], "captured");
    assert_eq!(
        row["probe_cert_sha256"],
        include_str!("../../test-fixtures/tls/leaf.sha256").trim()
    );
    assert_eq!(
        row["probe_cert_der_base64"],
        base64::engine::general_purpose::STANDARD
            .encode(include_bytes!("../../test-fixtures/tls/leaf.der"))
    );
    assert_eq!(row["probe_tls_verified"], "false");
    assert!(cached_probe_is_current(row));
    let mut old = row.clone();
    old.remove("probe_cert_capture");
    assert!(!cached_probe_is_current(&old));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn received_certificate_survives_a_failed_http_body_without_claiming_trust_or_web_liveness() {
    let (url, server) =
        serve_tls(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\nx");
    let options = options();
    let rules = native_content::ContentRules::from_options(&options);
    let row = [("link".into(), url)].into_iter().collect();
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .danger_accept_invalid_certs(true)
        .tls_info(true)
        .build()
        .unwrap();
    let result = probe_one(
        &row,
        &options,
        &client,
        &rules,
        &native_probe_control::ProbePacing::new(),
        1000.0,
        &AtomicBool::new(false),
    );
    server.join().unwrap();
    assert_eq!(
        result["probe_cert_sha256"],
        include_str!("../../test-fixtures/tls/leaf.sha256").trim()
    );
    assert_eq!(result["probe_cert_tls_verified"], "false");
    assert_ne!(result["probe_outcome"], "web_alive");
}

#[test]
fn strict_tls_is_not_disabled_to_obtain_a_certificate() {
    let (url, server) = serve_tls(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
    let mut options = options();
    options.strict_tls = true;
    let rules = native_content::ContentRules::from_options(&options);
    let row = [("link".into(), url)].into_iter().collect();
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .tls_info(true)
        .build()
        .unwrap();
    let result = probe_one(
        &row,
        &options,
        &client,
        &rules,
        &native_probe_control::ProbePacing::new(),
        1000.0,
        &AtomicBool::new(false),
    );
    server.join().unwrap();
    assert!(text(&result, &["probe_cert_sha256"]).is_empty());
    assert_ne!(result["probe_outcome"], "web_alive");
}

#[test]
fn redirect_host_mismatch_does_not_attach_a_valid_certificate_to_the_wrong_asset() {
    let (url, server) =
        serve_tls(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let response = reqwest::blocking::Client::builder()
        .no_proxy()
        .danger_accept_invalid_certs(true)
        .tls_info(true)
        .build()
        .unwrap()
        .get(&url)
        .send()
        .unwrap();
    let wrong = response_certificate(&response, "https://different.example/");
    assert_eq!(wrong["probe_cert_capture"], "redirect_host_changed");
    assert!(!wrong.contains_key("probe_cert_sha256"));
    server.join().unwrap();
}
