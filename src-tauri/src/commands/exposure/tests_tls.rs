//! Certificate failures must not hide authorized public pages, and must not weaken
//! authenticated provider requests. Every test uses a synthetic loopback TLS peer.
use super::adapters::http::{
    PassiveClient, PassiveHttpLimits, ProviderAuthorization, RequestMeter,
};
use super::fixture::{response, routes, Fixture};
use super::scope::{ExposureDenial, ExposureGrant, ExposureScope};
use base64::{engine::general_purpose::STANDARD, Engine};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

// Public synthetic test identity shared with auth_session_validation_tests.rs.
// It is not installed into the trust store. Both SANs identify loopback only.
const TEST_CERT_DER: &str = "MIIB0zCCAXigAwIBAgIUD/Y3Q763FXIDS7WIAusK5dqauV0wCgYIKoZIzj0EAwIwMTEvMC0GA1UEAwwmT3ZpcmFwdG9yIHN5bnRoZXRpYyBsb29wYmFjayB0ZXN0IG9ubHkwIBcNMjYxMDEzMDQwNzQ4WhgPMjEyNjA5MTkwNDA3NDhaMDExLzAtBgNVBAMMJk92aXJhcHRvciBzeW50aGV0aWMgbG9vcGJhY2sgdGVzdCBvbmx5MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAECrtWDMM9CMfa5RVarPsZTMVEzHOM7NdgbOofYVqrnQxV5PI1xleHiNTw795878CPG5zzgpQ9yf1UEa5r8Vqmj6NsMGowHQYDVR0OBBYEFCjL49wMBwkRYuyi8ObTtOsJFaL1MB8GA1UdIwQYMBaAFCjL49wMBwkRYuyi8ObTtOsJFaL1MBoGA1UdEQQTMBGHBH8AAAGCCWxvY2FsaG9zdDAMBgNVHRMBAf8EAjAAMAoGCCqGSM49BAMCA0kAMEYCIQDauA3mxuynd18A7CgmwB+mwgcvng2ZSgGzOQPOrkRbOwIhAN1Mrr9ay4FgXcIgHSWf4iOTiIOYcW7r6hWHAgHmWF7k";
const TEST_KEY_DER: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg8WWz//zCb+qBDDadbUKJfVFXrSl+O/wzfiOOKa7XIvChRANCAAQKu1YMwz0Ix9rlFVqs+xlMxUTMc4zs12Bs6h9hWqudDFXk8jXGV4eI1PDv3nzvwI8bnPOClD3J/VQRrmvxWqaP";

struct TlsPeer {
    origin: String,
    connections: Arc<AtomicUsize>,
    heads: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl TlsPeer {
    fn start(reply: String) -> Self {
        Self::scripted(vec![reply])
    }

    fn scripted(mut replies: Vec<String>) -> Self {
        assert!(!replies.is_empty());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let config = Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(
                    STANDARD.decode(TEST_CERT_DER).unwrap(),
                )],
                PrivatePkcs8KeyDer::from(STANDARD.decode(TEST_KEY_DER).unwrap()).into(),
            )
            .unwrap(),
        );
        let connections = Arc::new(AtomicUsize::new(0));
        let heads = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen_connections, seen_heads, stopping) =
            (connections.clone(), heads.clone(), stop.clone());
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let (socket, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("TLS fixture accept: {error}"),
                };
                seen_connections.fetch_add(1, Ordering::SeqCst);
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let connection = rustls::ServerConnection::new(config.clone()).unwrap();
                let mut stream = rustls::StreamOwned::new(connection, socket);
                let mut request = Vec::new();
                let mut buffer = [0u8; 2048];
                loop {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break, // Strict peer rejects our unknown issuer.
                        Ok(size) => request.extend_from_slice(&buffer[..size]),
                    }
                    if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        seen_heads
                            .lock()
                            .unwrap()
                            .push(String::from_utf8(request).unwrap());
                        let reply = if replies.len() > 1 {
                            replies.remove(0)
                        } else {
                            replies[0].clone()
                        };
                        let _ = stream.write_all(reply.as_bytes());
                        let _ = stream.flush();
                        break;
                    }
                    assert!(request.len() < 64 * 1024);
                }
            }
        });
        Self {
            origin: format!("https://{address}"),
            connections,
            heads,
            stop,
            worker: Some(worker),
        }
    }

    fn url(&self, path: &str) -> reqwest::Url {
        reqwest::Url::parse(&format!("{}{path}", self.origin)).unwrap()
    }

    fn finish(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }

    fn assert_receipts(&mut self, connections: usize, requests: usize) {
        self.finish();
        assert_eq!(
            self.connections.load(Ordering::SeqCst),
            connections,
            "actual TLS handshakes"
        );
        assert_eq!(
            self.heads.lock().unwrap().len(),
            requests,
            "actual HTTP requests"
        );
    }
}

impl Drop for TlsPeer {
    fn drop(&mut self) {
        self.finish();
    }
}

fn scope(peer: &TlsPeer) -> ExposureScope {
    ExposureScope {
        project_id: 71,
        scope_version: 1,
        issued_at: String::new(),
        valid_until: None,
        coarse_legacy: false,
        grants: vec![ExposureGrant::from_url(&peer.url("/")).unwrap()],
        sources: vec!["passive_web".into()],
        max_requests: 20,
        max_response_bytes: 4096,
        max_total_bytes: 8192,
        allow_private_destinations: true,
        allow_same_host_http_redirects: false,
        excluded: Vec::new(),
    }
}

fn client() -> PassiveClient {
    PassiveClient::new(PassiveHttpLimits {
        timeout: Duration::from_secs(3),
        max_response_bytes: 4096,
        ..PassiveHttpLimits::default()
    })
    .unwrap()
}

#[test]
fn tls_invalid_certificate_target_retries_once_with_warning_and_no_credentials() {
    let mut peer = TlsPeer::start(response(
        200,
        &[("Content-Type", "text/html")],
        "public page",
    ));
    let policy = scope(&peer);
    let mut meter = RequestMeter::for_scope(&policy);
    let result = client()
        .fetch(&policy, &peer.url("/"), &mut meter, &|| Ok(()), &|| false)
        .unwrap();
    assert_eq!(result.status, 200);
    assert_eq!(result.body, b"public page");
    assert_eq!(
        meter.requests_used, 2,
        "failed strict and fallback sends both spend budget"
    );
    let warning = result
        .tls_validation
        .as_ref()
        .expect("certificate anomaly retained");
    assert!(warning.validation_bypassed);
    assert!(!warning.reason_code.is_empty());
    assert!(!warning.diagnostic.is_empty());
    let serialized = serde_json::to_value(warning).unwrap();
    assert_eq!(
        serialized["validationBypassed"], true,
        "warning DTO uses camelCase"
    );
    peer.assert_receipts(2, 1);
    let head = peer.heads.lock().unwrap()[0].to_ascii_lowercase();
    assert!(head.starts_with("get / http/1.1"));
    assert!(!head.contains("authorization:"));
    assert!(!head.contains("cookie:"));
}

#[test]
fn tls_invalid_certificate_never_falls_back_for_sources_or_credentialed_providers() {
    for provider in [false, true] {
        let mut peer = TlsPeer::start(response(200, &[], "must not reach HTTP"));
        let mut meter = RequestMeter::for_scope(&scope(&peer));
        let fetcher = client();
        let origins = vec![peer.origin.clone()];
        let result = if provider {
            let secret = ProviderAuthorization::bearer(&peer.origin, "synthetic-provider-secret");
            fetcher.fetch_provider_api(
                &peer.url("/"),
                &origins,
                &[],
                Some(&secret),
                &mut meter,
                true,
                &|| false,
            )
        } else {
            fetcher.fetch_source_endpoint(&peer.url("/"), &origins, &mut meter, true, &|| false)
        };
        assert_eq!(result.unwrap_err().code(), "transport_failed");
        assert_eq!(meter.requests_used, 1);
        peer.assert_receipts(1, 0);
    }
}

#[test]
fn tls_invalid_certificate_fallback_obeys_remaining_request_budget() {
    let mut peer = TlsPeer::start(response(200, &[], "not reached"));
    let policy = scope(&peer);
    let mut meter = RequestMeter::for_scope(&policy);
    meter.max_requests = 1;
    let failure = client()
        .fetch(&policy, &peer.url("/"), &mut meter, &|| Ok(()), &|| false)
        .unwrap_err();
    assert_eq!(failure.code(), "request_budget_exhausted");
    assert!(failure.message().contains("tls_certificate_exception"));
    assert_eq!(meter.requests_used, 1);
    peer.assert_receipts(1, 0);
}

#[test]
fn tls_invalid_certificate_fallback_rechecks_cancellation_before_second_send() {
    let mut peer = TlsPeer::start(response(200, &[], "not reached"));
    let policy = scope(&peer);
    let mut meter = RequestMeter::for_scope(&policy);
    let checks = AtomicUsize::new(0);
    let cancelled = || checks.fetch_add(1, Ordering::SeqCst) > 0;
    let failure = client()
        .fetch(&policy, &peer.url("/"), &mut meter, &|| Ok(()), &cancelled)
        .unwrap_err();
    assert_eq!(failure.code(), "cancelled");
    assert_eq!(meter.requests_used, 1);
    peer.assert_receipts(1, 0);
}

#[test]
fn tls_invalid_certificate_fallback_rechecks_revocation_before_second_send() {
    let mut peer = TlsPeer::start(response(200, &[], "not reached"));
    let policy = scope(&peer);
    let mut meter = RequestMeter::for_scope(&policy);
    let checks = AtomicUsize::new(0);
    let revalidate = || {
        if checks.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(())
        } else {
            Err(ExposureDenial::new(
                "scope_revoked",
                "scope revoked during strict handshake",
            ))
        }
    };
    let failure = client()
        .fetch(&policy, &peer.url("/"), &mut meter, &revalidate, &|| false)
        .unwrap_err();
    assert_eq!(failure.code(), "scope_revoked");
    assert!(failure.message().contains("tls_certificate_exception"));
    assert_eq!(meter.requests_used, 1);
    peer.assert_receipts(1, 0);
}

#[test]
fn tls_invalid_certificate_fallback_does_not_follow_cross_origin_redirects() {
    let outside = Fixture::start(routes(vec![(
        "/outside",
        vec![response(200, &[], "not reached")],
    )]));
    let mut peer = TlsPeer::start(response(302, &[("Location", &outside.url("/outside"))], ""));
    let policy = scope(&peer);
    let mut meter = RequestMeter::for_scope(&policy);
    let failure = client()
        .fetch(&policy, &peer.url("/"), &mut meter, &|| Ok(()), &|| false)
        .unwrap_err();
    assert_eq!(failure.code(), "redirect_cross_origin");
    assert!(failure.message().contains("tls_certificate_exception"));
    assert!(failure.message().contains("TLS 身份未经验证"));
    assert_eq!(meter.requests_used, 2);
    peer.assert_receipts(2, 1);
    assert_eq!(outside.request_count(), 0);
}

#[test]
fn tls_invalid_certificate_warning_survives_incomplete_response_body() {
    let mut peer = TlsPeer::start(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 128\r\nConnection: close\r\n\r\nshort".into(),
    );
    let policy = scope(&peer);
    let mut meter = RequestMeter::for_scope(&policy);
    let failure = client()
        .fetch(&policy, &peer.url("/"), &mut meter, &|| Ok(()), &|| false)
        .unwrap_err();
    assert_eq!(
        failure.code(),
        "transport_failed",
        "body read failure retains its code"
    );
    assert!(failure.message().contains("tls_certificate_exception"));
    assert!(failure.message().contains("TLS 身份未经验证"));
    assert!(
        failure.message().contains("certificate"),
        "typed peer diagnostic retained"
    );
    assert_eq!(meter.requests_used, 2);
    peer.assert_receipts(2, 1);
}

#[test]
fn tls_invalid_certificate_warning_survives_response_byte_budget_refusal() {
    let mut peer = TlsPeer::start(response(
        200,
        &[("Content-Type", "text/plain")],
        "public body",
    ));
    let policy = scope(&peer);
    let mut meter = RequestMeter::for_scope(&policy);
    meter.max_bytes = 1;
    let failure = client()
        .fetch(&policy, &peer.url("/"), &mut meter, &|| Ok(()), &|| false)
        .unwrap_err();
    assert_eq!(
        failure.code(),
        "byte_budget_exhausted",
        "quota refusal retains its code"
    );
    assert!(failure.message().contains("tls_certificate_exception"));
    assert!(failure.message().contains("TLS 身份未经验证"));
    assert_eq!(meter.requests_used, 2);
    peer.assert_receipts(2, 1);
}

#[test]
fn tls_invalid_certificate_same_origin_redirect_remains_scoped_and_metered() {
    let mut peer = TlsPeer::scripted(vec![
        response(302, &[("Location", "/public")], ""),
        response(
            200,
            &[("Content-Type", "text/plain")],
            "public redirected page",
        ),
    ]);
    let policy = scope(&peer);
    let mut meter = RequestMeter::for_scope(&policy);
    let result = client()
        .fetch(&policy, &peer.url("/"), &mut meter, &|| Ok(()), &|| false)
        .unwrap();
    assert_eq!(result.status, 200);
    assert_eq!(result.final_url, peer.url("/public").to_string());
    assert!(result.tls_validation.as_ref().unwrap().validation_bypassed);
    assert_eq!(
        meter.requests_used, 4,
        "strict and fallback counted separately at both hops"
    );
    peer.assert_receipts(4, 2);
    let heads = peer.heads.lock().unwrap();
    assert!(heads[0].starts_with("GET / HTTP/1.1"));
    assert!(heads[1].starts_with("GET /public HTTP/1.1"));
}

#[test]
fn tls_warning_survives_quota_exhaustion_before_same_origin_redirect_hop() {
    let mut peer = TlsPeer::start(response(302, &[("Location", "/public")], ""));
    let policy = scope(&peer);
    let mut meter = RequestMeter::for_scope(&policy);
    meter.max_requests = 2;
    let failure = client()
        .fetch(&policy, &peer.url("/"), &mut meter, &|| Ok(()), &|| false)
        .unwrap_err();
    assert_eq!(failure.code(), "request_budget_exhausted");
    assert!(failure.message().contains("tls_certificate_exception"));
    assert!(failure.message().contains("TLS 身份未经验证"));
    assert_eq!(meter.requests_used, 2);
    peer.assert_receipts(2, 1);
}

#[test]
fn tls_warning_survives_revocation_before_same_origin_redirect_hop() {
    let mut peer = TlsPeer::start(response(302, &[("Location", "/public")], ""));
    let policy = scope(&peer);
    let mut meter = RequestMeter::for_scope(&policy);
    let checks = AtomicUsize::new(0);
    let revalidate = || {
        if checks.fetch_add(1, Ordering::SeqCst) < 2 {
            Ok(())
        } else {
            Err(ExposureDenial::new(
                "scope_revoked",
                "revoked after public redirect response",
            ))
        }
    };
    let failure = client()
        .fetch(&policy, &peer.url("/"), &mut meter, &revalidate, &|| false)
        .unwrap_err();
    assert_eq!(failure.code(), "scope_revoked");
    assert!(failure.message().contains("tls_certificate_exception"));
    assert!(failure.message().contains("TLS 身份未经验证"));
    assert_eq!(meter.requests_used, 2);
    peer.assert_receipts(2, 1);
}

#[test]
fn tls_walker_saves_certificate_warning_and_counts_fallback_against_origin_ceiling() {
    use super::adapters::passive_web::{execute, WalkBudget};
    use super::state::WorkItemStatus;
    use super::work_items::WorkItem;
    let mut peer = TlsPeer::start(response(
        200,
        &[("Content-Type", "text/plain")],
        "public page",
    ));
    let policy = scope(&peer);
    let mut meter = RequestMeter::for_scope(&policy);
    let item = WorkItem {
        id: 1,
        run_id: 2,
        project_id: policy.project_id,
        work_key: "synthetic-tls-origin".into(),
        kind: "passive_web".into(),
        source_key: "passive_web".into(),
        asset_id: Some(3),
        origin: peer.origin.clone(),
        entity: peer.origin.clone(),
        input_hash: "synthetic-tls".into(),
        status: WorkItemStatus::InProgress,
        owner: "tls-test".into(),
        claim_version: 1,
        attempts: 1,
        observations: 0,
        requests: 0,
        bytes: 0,
    };
    let outcome = execute(
        &client(),
        &policy,
        &item,
        &mut meter,
        &WalkBudget {
            max_attempts: 3,
            max_queued: 10,
        },
        &|| Ok(()),
        &|| false,
    );
    assert_eq!(outcome.status, Some(WorkItemStatus::BlockedQuota));
    assert_eq!(
        meter.requests_used, 3,
        "origin ceiling includes failed TLS handshakes"
    );
    assert_eq!(outcome.requests, 3);
    let page = outcome
        .observations
        .iter()
        .find(|row| row.status_code == 200)
        .expect("public page retained");
    assert_eq!(page.extra["tlsValidation"]["validationBypassed"], true);
    assert_eq!(
        page.extra["tlsValidation"]["scope"],
        "authorized_public_target"
    );
    assert!(outcome.source_reason.contains("TLS"));
    peer.assert_receipts(3, 1);
}

#[test]
fn tls_noncertificate_handshake_failures_do_not_trigger_tolerant_retry() {
    // This plain HTTP peer cannot complete TLS, so the failure has no invalid-certificate cause.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let stopping = Arc::new(AtomicBool::new(false));
    let stop = stopping.clone();
    let connections = Arc::new(AtomicUsize::new(0));
    let seen = connections.clone();
    let worker = thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    seen.fetch_add(1, Ordering::SeqCst);
                    let _ =
                        socket.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(error) => panic!("plain TLS failure fixture: {error}"),
            }
        }
    });
    let identity_peer = TlsPeer::start(response(200, &[], "unused"));
    let mut policy = scope(&identity_peer);
    let url = reqwest::Url::parse(&format!("https://{address}/")).unwrap();
    policy.grants = vec![ExposureGrant::from_url(&url).unwrap()];
    let mut meter = RequestMeter::for_scope(&policy);
    let result = client().fetch(&policy, &url, &mut meter, &|| Ok(()), &|| false);
    stopping.store(true, Ordering::SeqCst);
    worker.join().unwrap();
    assert_eq!(result.unwrap_err().code(), "transport_failed");
    assert_eq!(meter.requests_used, 1);
    assert_eq!(connections.load(Ordering::SeqCst), 1);
}

// The proxy is loopback-only: declared target origins still use exact ports 443/80,
// while no test needs to bind privileged ports or resolve an external hostname.
const EXPIRED_CERT_DER: &str = "MIIBgTCCASagAwIBAgIBATAKBggqhkjOPQQDAjA0MTIwMAYDVQQDDClPdmlyYXB0b3Igc3ludGhldGljIGV4cGlyZWQgbG9vcGJhY2sgb25seTAeFw0yMDAxMDEwMDAwMDBaFw0yMDAxMDIwMDAwMDBaMDQxMjAwBgNVBAMMKU92aXJhcHRvciBzeW50aGV0aWMgZXhwaXJlZCBsb29wYmFjayBvbmx5MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEq1lHjJ8PGAZR20kpobXG89zvmTrUnd3R/wyk2PNKRhjPK7bmzYA/QFc7N4F3YlM2g8+KquF0QRwJK5tR2PQRrqMpMCcwGgYDVR0RBBMwEYcEfwAAAYIJbG9jYWxob3N0MAkGA1UdEwQCMAAwCgYIKoZIzj0EAwIDSQAwRgIhAJ5hDQfICIRUhmjGAECVUvwSsLmqJ/SCDbxCrL2zOb7FAiEAglHq6nk8WMPI3r5Tbhb4W99UQXOQBgP6R1a4QcE7iBw=";
const EXPIRED_KEY_DER: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgRW46FNoljsfbmhixYPl8wZ7iVhTZU2A0ee3qwnukjW+hRANCAASrWUeMnw8YBlHbSSmhtcbz3O+ZOtSd3dH/DKTY80pGGM8rtubNgD9AVzs3gXdiUzaDz4qq4XRBHAkrm1HY9BGu";

struct SameHostRedirectProxy {
    endpoint: String,
    heads: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

fn fixture_request_head(reader: &mut impl Read) -> Option<String> {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while bytes.len() < 64 * 1024 {
        if reader.read(&mut byte).ok()? == 0 { return None; }
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") { return String::from_utf8(bytes).ok(); }
    }
    None
}

impl SameHostRedirectProxy {
    fn start(location: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let config = Arc::new(rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        )).with_safe_default_protocol_versions().unwrap().with_no_client_auth()
          .with_single_cert(vec![CertificateDer::from(STANDARD.decode(EXPIRED_CERT_DER).unwrap())],
              PrivatePkcs8KeyDer::from(STANDARD.decode(EXPIRED_KEY_DER).unwrap()).into()).unwrap());
        let heads = Arc::new(Mutex::new(Vec::new()));
        let seen = heads.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let location = location.to_owned();
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let (mut socket, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2)); continue;
                    }
                    Err(error) => panic!("redirect proxy accept: {error}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                socket.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
                let Some(head) = fixture_request_head(&mut socket) else { continue; };
                seen.lock().unwrap().push(head.clone());
                if head.starts_with("CONNECT ") {
                    socket.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").unwrap();
                    let connection = rustls::ServerConnection::new(config.clone()).unwrap();
                    let mut tls = rustls::StreamOwned::new(connection, socket);
                    let Some(head) = fixture_request_head(&mut tls) else { continue; };
                    seen.lock().unwrap().push(head);
                    let _ = tls.write_all(response(302, &[("Location", &location)], "").as_bytes());
                    let _ = tls.flush();
                } else {
                    let _ = socket.write_all(response(200, &[("Content-Type", "text/html")], "public http destination").as_bytes());
                    let _ = socket.flush();
                }
            }
        });
        Self { endpoint, heads, stop, worker: Some(worker) }
    }

    fn client(&self) -> PassiveClient {
        PassiveClient::new(PassiveHttpLimits { proxy: Some(self.endpoint.clone()),
            timeout: Duration::from_secs(3), max_response_bytes: 4096, ..PassiveHttpLimits::default()
        }).unwrap()
    }

    fn http_requests(&self) -> usize {
        self.heads.lock().unwrap().iter().filter(|head| head.starts_with("GET http://")).count()
    }
}

impl Drop for SameHostRedirectProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() { worker.join().unwrap(); }
    }
}

fn default_port_scope() -> (ExposureScope, reqwest::Url) {
    let anchor = reqwest::Url::parse("https://127.0.0.1/").unwrap();
    let identity = TlsPeer::start(response(200, &[], "unused"));
    let mut policy = scope(&identity);
    policy.grants = vec![ExposureGrant::from_url(&anchor).unwrap()];
    policy.allow_same_host_http_redirects = true;
    (policy, anchor)
}

#[test]
fn tls_expired_redirect_to_explicit_same_host_http_collects_without_credentials_and_keeps_both_warnings() {
    let proxy = SameHostRedirectProxy::start("http://127.0.0.1/public");
    let (policy, anchor) = default_port_scope();
    let mut meter = RequestMeter::for_scope(&policy);
    let charged = AtomicUsize::new(0);
    let result = proxy.client().fetch_navigation_with_charge(&policy, &anchor, &anchor, &mut meter,
        &|| Ok(()), &|| false, &|| { charged.fetch_add(1, Ordering::SeqCst); Ok(()) }).unwrap();
    assert_eq!(result.status, 200);
    assert_eq!(result.body, b"public http destination");
    assert_eq!(result.final_url, "http://127.0.0.1/public");
    assert_eq!(result.requests_charged, 3);
    assert_eq!(meter.requests_used, 3);
    assert_eq!(charged.load(Ordering::SeqCst), 3);
    assert!(result.tls_validation.as_ref().unwrap().validation_bypassed);
    let warning = serde_json::to_value(result.http_downgrade.unwrap()).unwrap();
    assert_eq!(warning["status"], "same_host_http");
    assert_eq!(warning["transportEncrypted"], false);
    assert_eq!(warning["scope"], "authorized_same_host_public_get");
    assert_eq!(proxy.http_requests(), 1);
    for head in proxy.heads.lock().unwrap().iter() {
        let lower = head.to_ascii_lowercase();
        assert!(!lower.contains("authorization:"));
        assert!(!lower.contains("cookie:"));
    }
}

#[test]
fn tls_same_host_http_requires_opt_in_exact_host_and_default_port() {
    for (location, enabled) in [("http://127.0.0.1/public", false),
        ("http://localhost/public", true), ("http://127.0.0.1:8080/public", true),
        ("http://127.0.0.2/public", true), ("http://user:pass@127.0.0.1/public", true)] {
        let proxy = SameHostRedirectProxy::start(location);
        let (mut policy, anchor) = default_port_scope();
        policy.allow_same_host_http_redirects = enabled;
        let mut meter = RequestMeter::for_scope(&policy);
        let failure = proxy.client().fetch(&policy, &anchor, &mut meter, &|| Ok(()), &|| false).unwrap_err();
        assert!(matches!(failure.code(), "redirect_cross_origin" | "userinfo_denied"), "{}: {}", location, failure.message());
        assert!(failure.message().contains("tls_certificate_exception"));
        assert_eq!(meter.requests_used, 2);
        assert_eq!(proxy.http_requests(), 0);
    }
}

#[test]
fn tls_same_host_http_rechecks_revoke_pause_and_durable_charge_before_plaintext_send() {
    for guard in ["revoke", "pause", "charge"] {
        let proxy = SameHostRedirectProxy::start("http://127.0.0.1/public");
        let (policy, anchor) = default_port_scope();
        let mut meter = RequestMeter::for_scope(&policy);
        let checks = AtomicUsize::new(0);
        let cancel_checks = AtomicUsize::new(0);
        let charges = AtomicUsize::new(0);
        let result = proxy.client().fetch_navigation_with_charge(&policy, &anchor, &anchor, &mut meter,
            &|| if checks.fetch_add(1, Ordering::SeqCst) >= 2 && guard == "revoke" {
                Err(ExposureDenial::new("scope_revoked", "revoked after redirect"))
            } else { Ok(()) },
            &|| cancel_checks.fetch_add(1, Ordering::SeqCst) >= 2 && guard == "pause",
            &|| if charges.fetch_add(1, Ordering::SeqCst) >= 2 && guard == "charge" {
                Err(ExposureDenial::new("checkpoint_write_failed", "durable reservation rejected"))
            } else { Ok(()) });
        assert_eq!(result.unwrap_err().code(), match guard { "revoke" => "scope_revoked", "pause" => "cancelled", _ => "checkpoint_write_failed" });
        assert_eq!(proxy.http_requests(), 0, "{guard}");
        assert_eq!(meter.requests_used, 2, "no third send when {guard} gate rejects");
    }
}

#[test]
fn tls_same_host_http_scope_retains_private_gate_and_old_scope_identity_by_default() {
    use super::settings::ExposureSettings;
    let (mut policy, anchor) = default_port_scope();
    let http = reqwest::Url::parse("http://127.0.0.1/public").unwrap();
    policy.allow_private_destinations = false;
    assert_eq!(policy.authorize_navigation(&anchor, &http, "2026-10-24 00:00:00").unwrap_err().code, "private_destination_denied");
    policy.allow_private_destinations = true;
    assert!(policy.navigation_allowed(&anchor, &http));
    assert!(policy.navigation_allowed(&anchor, &anchor));
    assert!(!policy.navigation_allowed(&http, &anchor), "an ungranted HTTP anchor cannot invent HTTPS authorization");
    let alternate = reqwest::Url::parse("https://127.0.0.1:8443/").unwrap();
    policy.grants.push(ExposureGrant::from_url(&alternate).unwrap());
    assert!(!policy.navigation_allowed(&alternate, &http));
    assert_eq!(policy.as_json()["allowSameHostHttpRedirects"], true);
    policy.allow_same_host_http_redirects = false;
    assert!(policy.as_json().get("allowSameHostHttpRedirects").is_none());
    let defaults = ExposureSettings::for_project(71);
    assert!(!defaults.allow_same_host_http_redirects);
    assert!(defaults.to_settings_json().get("exposureAllowSameHostHttpRedirects").is_none());
    assert!(ExposureSettings::from_json(71, &serde_json::json!({"exposureAllowSameHostHttpRedirects":"true"})).is_err());
    let enabled = ExposureSettings::from_json(71, &serde_json::json!({"exposureAllowSameHostHttpRedirects":true})).unwrap();
    assert!(ExposureSettings::from_frozen_strict(71, 1, &enabled.to_settings_json().to_string()).unwrap().allow_same_host_http_redirects);
}

#[test]
fn tls_same_host_http_follows_natural_links_only_with_the_original_https_anchor() {
    let proxy = SameHostRedirectProxy::start("http://127.0.0.1/public");
    let (policy, anchor) = default_port_scope();
    let http = reqwest::Url::parse("http://127.0.0.1/next").unwrap();
    let mut meter = RequestMeter::for_scope(&policy);
    let fetcher = proxy.client();
    assert_eq!(fetcher.fetch(&policy, &http, &mut meter, &|| Ok(()), &|| false).unwrap_err().code(), "host_not_granted");
    assert_eq!(meter.requests_used, 0);
    let result = fetcher.fetch_navigation(&policy, &anchor, &http, &mut meter, &|| Ok(()), &|| false).unwrap();
    assert_eq!(result.status, 200);
    assert_eq!(meter.requests_used, 1);
    assert!(result.http_downgrade.is_some());
}


#[test]
fn http_total_deadline_stops_slow_drip_bodies_instead_of_resetting_each_read() {
    for mode in ["public", "source", "provider"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let (mut socket, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind()==std::io::ErrorKind::WouldBlock => { thread::sleep(Duration::from_millis(2)); continue; }
                    Err(error) => panic!("slow drip accept: {error}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
                socket.set_write_timeout(Some(Duration::from_millis(300))).unwrap();
                if fixture_request_head(&mut socket).is_none() { continue; }
                if socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 40\r\nConnection: close\r\n\r\n").is_err() { break; }
                // Each read arrives well inside the 100ms inactivity timeout. A
                // per-read-only timeout would therefore keep this body alive for 1s.
                for _ in 0..40 {
                    if stopping.load(Ordering::SeqCst) || socket.write_all(b"x").is_err() { break; }
                    let _ = socket.flush();
                    thread::sleep(Duration::from_millis(25));
                }
                break;
            }
        });
        let url = reqwest::Url::parse(&format!("http://{address}/slow")).unwrap();
        let (mut policy, _) = default_port_scope();
        policy.grants = vec![ExposureGrant::from_url(&url).unwrap()];
        let fetcher = PassiveClient::new(PassiveHttpLimits { timeout: Duration::from_millis(100), max_response_bytes: 4096, ..PassiveHttpLimits::default() }).unwrap();
        let mut meter = RequestMeter::for_scope(&policy);
        let origins = vec![url.origin().ascii_serialization()];
        let started = std::time::Instant::now();
        let result = match mode {
            "source" => fetcher.fetch_source_endpoint(&url, &origins, &mut meter, true, &|| false),
            "provider" => fetcher.fetch_provider_api(&url, &origins, &[], None, &mut meter, true, &|| false),
            _ => fetcher.fetch(&policy, &url, &mut meter, &|| Ok(()), &|| false),
        };
        let elapsed = started.elapsed();
        stop.store(true, Ordering::SeqCst);
        worker.join().unwrap();
        assert_eq!(result.unwrap_err().code(), "transport_failed", "{mode}");
        assert!(elapsed < Duration::from_millis(700), "{mode}: total body deadline did not fire: {elapsed:?}");
        assert_eq!(meter.requests_used, 1);
    }
}
