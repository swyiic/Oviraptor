// Dependency-free test HTTP fixture. It is also compiled directly with rustc
// for low-load lifecycle regression checks without rebuilding the application.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;

type Seen = Arc<Mutex<Vec<String>>>;
type Handler = Arc<dyn Fn(String) -> (u16, &'static str, String) + Send + Sync + 'static>;
type ResponseHandler = Arc<dyn Fn(String) -> String + Send + Sync + 'static>;

pub(crate) fn read_http_request(stream: &mut TcpStream) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut header_end = None;
    let mut want = 0usize;
    let mut continued = false;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(size) => {
                buffer.extend_from_slice(&chunk[..size]);
                if header_end.is_none() {
                    header_end = String::from_utf8_lossy(&buffer).find("\r\n\r\n");
                    if let Some(index) = header_end {
                        let head = String::from_utf8_lossy(&buffer[..index]).to_ascii_lowercase();
                        want = head
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length:")
                                    .map(|rest| rest.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                    }
                }
                let Some(index) = header_end else { continue };
                if buffer.len() >= index + 4 + want {
                    break;
                }
                if !continued
                    && String::from_utf8_lossy(&buffer[..index])
                        .to_ascii_lowercase()
                        .contains("expect: 100-continue")
                {
                    let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
                    continued = true;
                }
            }
            Err(_) => break,
        }
    }
    header_end?;
    Some(String::from_utf8_lossy(&buffer).to_string())
}

pub(crate) fn spawn_endpoint(handler: Handler) -> (u16, Seen, Arc<AtomicBool>) {
    spawn_response_endpoint(Arc::new(move |request| {
        let (status, content_type, body) = handler(request);
        format!("HTTP/1.1 {status} OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
    }))
}

// Redirect fixtures need a Location header, while the proxy canary must refuse
// CONNECT. Both use the same caller-owned listener lifetime as model fixtures.
pub(crate) fn spawn_response_endpoint(handler: ResponseHandler) -> (u16, Seen, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    // Neither the listener nor request workers own a caller's lifecycle guard.
    // Some fixtures retain Seen instead of stop; either keeps the endpoint
    // alive. An explicit stop wins even while a history reader remains alive.
    let loop_stop = Arc::downgrade(&stop);
    let loop_seen = Arc::downgrade(&seen);
    thread::Builder::new().name("test-http-listener".into()).spawn(move || {
        while loop_stop.strong_count() > 0 || loop_seen.strong_count() > 0 {
            if loop_stop.upgrade().is_some_and(|stop| stop.load(Ordering::SeqCst)) { break; }
            let (stream, _) = match listener.accept() {
                Ok(accepted) => accepted,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            // macOS can inherit O_NONBLOCK from the listener. Read complete
            // requests on the accepted stream rather than dropping/retrying.
            let _ = stream.set_nonblocking(false);
            let seen = loop_seen.clone();
            let stop = loop_stop.clone();
            let handler = handler.clone();
            thread::Builder::new().name("test-http-request".into()).spawn(move || {
                if stop.upgrade().is_some_and(|stop| stop.load(Ordering::SeqCst)) { return; }
                let mut stream = stream;
                let Some(request) = read_http_request(&mut stream) else { return };
                let response = handler(request.clone());
                if let Some(seen) = seen.upgrade() { seen.lock().unwrap().push(request); }
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }).unwrap();
        }
    }).unwrap();
    (port, seen, stop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn request(port: u16) -> String {
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nhost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        response
    }

    fn wait_released(port: u16) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            // Rebinding proves the listening socket has actually been dropped;
            // an ownership counter alone would not establish thread cleanup.
            if TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "test endpoint leaked listener {port}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn endpoint_releases_listener_and_handler_when_owners_drop() {
        let handler: Handler = Arc::new(|_| (200, "text/plain", "ok".into()));
        let weak_handler = Arc::downgrade(&handler);
        let (port, seen, stop) = spawn_endpoint(handler);
        assert_eq!(Arc::strong_count(&seen), 1);
        assert_eq!(Arc::strong_count(&stop), 1);
        drop(seen);
        drop(stop);
        wait_released(port);
        let deadline = Instant::now() + Duration::from_secs(2);
        while weak_handler.strong_count() != 0 {
            assert!(Instant::now() < deadline, "listener still owns handler");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn endpoint_history_owner_supports_existing_harness_without_stop_handle() {
        let (port, seen, stop) = spawn_endpoint(Arc::new(|_| (200, "text/plain", "ok".into())));
        drop(stop);
        assert!(request(port).ends_with("ok"));
        assert_eq!(seen.lock().unwrap().len(), 1);
        // A prior accepted TCP connection can keep a port in TIME_WAIT. Use
        // connection refusal instead of requiring an immediate bind here.
        drop(seen);
        let deadline = Instant::now() + Duration::from_secs(2);
        while TcpStream::connect(("127.0.0.1", port)).is_ok() {
            assert!(Instant::now() < deadline, "history-only listener leaked");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn endpoint_explicit_stop_overrides_history_owner() {
        let (port, seen, stop) = spawn_endpoint(Arc::new(|_| (200, "text/plain", "ok".into())));
        stop.store(true, Ordering::SeqCst);
        wait_released(port);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn endpoint_stop_owner_can_serve_without_history_owner() {
        let (port, seen, stop) = spawn_endpoint(Arc::new(|_| (200, "text/plain", "ok".into())));
        drop(seen);
        assert!(request(port).ends_with("ok"));
        stop.store(true, Ordering::SeqCst);
    }

    #[test]
    fn endpoint_many_dropped_fixtures_release_their_listeners() {
        let mut ports = Vec::new();
        for _ in 0..32 {
            let (port, seen, stop) = spawn_endpoint(Arc::new(|_| (200, "text/plain", "ok".into())));
            ports.push(port);
            drop(seen);
            drop(stop);
        }
        for port in ports {
            wait_released(port);
        }
    }
}
