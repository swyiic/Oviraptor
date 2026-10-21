//! Hunter-only connection policy. Never modifies VPN, routes, DNS or interfaces.
use super::*;

pub(super) fn mode(c: &Connection) -> Result<String, String> {
    let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_discovery_hunter_network' AND type='table')", [], |r| r.get(0)).map_err(err)?;
    if !exists {
        return Ok("direct".into());
    }
    Ok(c.query_row(
        "SELECT mode FROM asset_discovery_hunter_network WHERE id=1",
        [],
        |r| r.get(0),
    )
    .optional()
    .map_err(err)?
    .unwrap_or_else(|| "direct".into()))
}

pub(super) fn client(
    c: &Connection,
    settings: &Value,
    seconds: u64,
) -> Result<reqwest::blocking::Client, String> {
    let mut b = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(seconds))
        .redirect(reqwest::redirect::Policy::none());
    if mode(c)? == "business" {
        if let Some(proxy) = settings["proxyUrl"].as_str().filter(|p| !p.is_empty()) {
            b = b.proxy(reqwest::Proxy::all(proxy).map_err(|_| "Hunter 业务代理无效")?);
        }
    } else {
        // Request-local policy only. DNS and routing remain OS-owned.
        // A VPN in TUN/global mode can still intercept traffic; the user closes it for testing.
        b = b.no_proxy();
    }
    b.build().map_err(|_| "无法初始化 Hunter 网络连接".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_settings_read_defaults_to_direct_without_writes() {
        let c = Connection::open_in_memory().unwrap();
        assert_eq!(mode(&c).unwrap(), "direct");
        let count: i64 = c
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    #[test]
    fn direct_client_ignores_business_proxy_and_preserves_redirect_refusal() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut b = [0u8; 1024];
            let _ = socket.read(&mut b);
            std::io::Write::write_all(&mut socket,b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/must-not-follow\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let c = Connection::open_in_memory().unwrap();
        let transport = client(&c, &json!({"proxyUrl":"http://127.0.0.1:1"}), 2).unwrap();
        assert_eq!(
            transport
                .get(format!("http://{address}/"))
                .send()
                .unwrap()
                .status()
                .as_u16(),
            302
        );
        worker.join().unwrap();
    }
}
