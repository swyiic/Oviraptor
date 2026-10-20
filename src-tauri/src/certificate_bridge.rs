//! Private loopback bridge. Only the trusted desktop bootstrap gives the Host its bearer.
use crate::asset_discovery::certificate_tasks::Manager;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
pub(crate) struct Bridge {
    pub origin: String,
    pub token: String,
    stop: Arc<AtomicBool>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Call {
    project_id: i64,
    action: String,
    args: Value,
    operation_id: String,
}
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |v, (a, b)| v | (a ^ b)) == 0
}
fn read(stream: &mut TcpStream, token: &str) -> Result<Call, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| "bridge_io")?;
    let mut buf = Vec::new();
    let mut block = [0; 4096];
    let end = loop {
        if buf.len() > 40 * 1024 {
            return Err("bridge_frame_too_large".into());
        }
        let n = stream.read(&mut block).map_err(|_| "bridge_io")?;
        if n == 0 {
            return Err("bridge_frame_invalid".into());
        }
        buf.extend_from_slice(&block[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let header = std::str::from_utf8(&buf[..end]).map_err(|_| "bridge_frame_invalid")?;
    let mut lines = header.split("\r\n");
    if lines.next() != Some("POST /certificate-task HTTP/1.1") {
        return Err("bridge_route_invalid".into());
    }
    let mut length = None;
    let mut authorized = false;
    for line in lines.filter(|s| !s.is_empty()) {
        let (name, value) = line.split_once(':').ok_or("bridge_frame_invalid")?;
        match name.to_ascii_lowercase().as_str() {
            "authorization" => {
                if authorized {
                    return Err("bridge_auth_invalid".into());
                }
                authorized = same(value.trim(), &format!("Bearer {token}"));
            }
            "origin" | "transfer-encoding" => return Err("bridge_origin_denied".into()),
            "content-length" => {
                if length.is_some() {
                    return Err("bridge_frame_invalid".into());
                }
                length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| "bridge_frame_invalid")?,
                );
            }
            _ => {}
        }
    }
    if !authorized {
        return Err("bridge_auth_invalid".into());
    }
    let size = length
        .filter(|n| *n <= 32 * 1024)
        .ok_or("bridge_frame_invalid")?;
    while buf.len() < end + size {
        let n = stream.read(&mut block).map_err(|_| "bridge_io")?;
        if n == 0 {
            return Err("bridge_frame_invalid".into());
        }
        buf.extend_from_slice(&block[..n]);
        if buf.len() > end + 32 * 1024 {
            return Err("bridge_frame_too_large".into());
        }
    }
    if buf.len() != end + size {
        return Err("bridge_frame_invalid".into());
    }
    let c: Call = serde_json::from_slice(&buf[end..]).map_err(|_| "bridge_frame_invalid")?;
    if c.project_id < 1 || c.operation_id.is_empty() || c.operation_id.len() > 128 {
        return Err("bridge_context_invalid".into());
    }
    Ok(c)
}
fn handle(mut stream: TcpStream, token: &str, manager: &Manager) {
    let outcome = read(&mut stream, token)
        .and_then(|c| manager.dispatch(c.project_id, &c.action, c.args, &c.operation_id));
    let (status, value) = match outcome {
        Ok(v) => (200, json!({"ok":true,"result":v})),
        Err(e) => (400, json!({"ok":false,"error":e})),
    };
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let _=write!(stream,"HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",body.len());
    let _ = stream.write_all(&body);
}
impl Bridge {
    pub(crate) fn start(manager: Manager) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let origin = format!(
            "http://{}",
            listener.local_addr().map_err(|e| e.to_string())?
        );
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let key = token.clone();
        thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => handle(stream, &key, &manager),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(30))
                    }
                    Err(_) => break,
                }
            }
            manager.shutdown();
        });
        Ok(Self {
            origin,
            token,
            stop,
        })
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst)
    }
}
