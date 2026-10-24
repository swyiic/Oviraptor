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
