//! Certificate-only TLS handshake: no HTTP request, redirects, page reads or credentials.
//! The observed leaf is evidence, never an assertion of CA trust or ownership.
use crate::certificates::{self, Certificate};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use std::{net::IpAddr, sync::Arc, time::Duration};
#[derive(Debug)]
struct ObserveCertificate;
impl ServerCertVerifier for ObserveCertificate {
    fn verify_server_cert(
        &self,
        leaf: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        certificates::from_der(leaf.as_ref()).ok_or(rustls::Error::InvalidCertificate(
            rustls::CertificateError::BadEncoding,
        ))?;
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            m,
            c,
            s,
            &rustls::crypto::aws_lc_rs::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            m,
            c,
            s,
            &rustls::crypto::aws_lc_rs::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
pub(crate) fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            !v.is_private()
                && !v.is_loopback()
                && !v.is_link_local()
                && !v.is_unspecified()
                && !v.is_broadcast()
                && !v.is_multicast()
                && !v.is_documentation()
                && v.octets()[0] != 0
                && v.octets()[0] < 240
                && !(v.octets()[0] == 100 && (64..=127).contains(&v.octets()[1]))
                && !(v.octets()[0] == 198 && (18..=19).contains(&v.octets()[1]))
        }
        IpAddr::V6(v) => {
            !v.is_loopback()
                && !v.is_unspecified()
                && !v.is_unique_local()
                && !v.is_unicast_link_local()
                && !v.is_multicast()
                && v.to_ipv4_mapped()
                    .map(|i| public(IpAddr::V4(i)))
                    .unwrap_or(true)
        }
    }
}
pub(crate) async fn capture(
    url: &str,
    seconds: u64,
    allow_private: bool,
) -> Result<Certificate, String> {
    capture_at(url, seconds, allow_private, None).await
}
pub(crate) async fn capture_at(
    url: &str,
    seconds: u64,
    allow_private: bool,
    pinned: Option<Vec<IpAddr>>,
) -> Result<Certificate, String> {
    let u = reqwest::Url::parse(url).map_err(|_| "invalid_endpoint")?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() {
        return Err("invalid_endpoint".into());
    }
    let host = u
        .host_str()
        .ok_or("invalid_endpoint")?
        .trim_matches(['[', ']'])
        .to_owned();
    let port = u.port_or_known_default().ok_or("invalid_endpoint")?;
    let operation = async {
        let addresses: Vec<std::net::SocketAddr> = if let Some(ips) = pinned {
            ips.into_iter()
                .map(|ip| std::net::SocketAddr::new(ip, port))
                .collect()
        } else {
            tokio::net::lookup_host((host.as_str(), port))
                .await
                .map_err(|e| format!("dns_failed: {e}"))?
                .collect()
        };
        let mut socket = None;
        let mut allowed = false;
        let mut connection_error = String::new();
        for address in addresses.into_iter().take(8) {
            if !allow_private && !public(address.ip()) {
                continue;
            }
            allowed = true;
            match tokio::net::TcpStream::connect(address).await {
                Ok(stream) => { socket=Some(stream);break; }
                Err(e) => connection_error=format!("{address}: {e}"),
            }
        }
        if !allowed {
            return Err("non_public_address".into());
        }
        let socket = socket.ok_or_else(|| format!("connect_failed: {connection_error}"))?;
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| "tls_config_failed")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ObserveCertificate))
        .with_no_client_auth();
        let name = ServerName::try_from(host).map_err(|_| "invalid_endpoint")?;
        let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, socket)
            .await
            .map_err(|e| format!("tls_handshake_failed: {e}"))?;
        let der = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|chain| chain.first())
            .ok_or("certificate_missing")?;
        certificates::from_der(der.as_ref()).ok_or_else(|| "certificate_invalid".into())
    };
    tokio::time::timeout(Duration::from_secs(seconds), operation)
        .await
        .map_err(|_| "timeout".to_string())?
}
