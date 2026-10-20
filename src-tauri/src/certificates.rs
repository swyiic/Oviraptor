//! Certificate identity is SHA-256 of the complete leaf X.509 DER, never its
//! subject, serial, public key, PEM text, HTTP body or certificate chain.
use base64::{engine::general_purpose::STANDARD, Engine};
use rustls::pki_types::{pem::PemObject, CertificateDer};
use sha2::{Digest, Sha256};

pub(crate) struct Certificate {
    pub sha256: String,
    pub der_base64: String,
    pub dns_names: Vec<String>,
}

pub(crate) fn from_der(der: &[u8]) -> Option<Certificate> {
    if der.is_empty() || der.len() > 64 * 1024 {
        return None;
    }
    let bytes = CertificateDer::from(der);
    let parsed = webpki::EndEntityCert::try_from(&bytes).ok()?;
    Some(Certificate {
        sha256: format!("{:x}", Sha256::digest(der)),
        der_base64: STANDARD.encode(der),
        dns_names: parsed
            .valid_dns_names()
            .take(128)
            .map(str::to_owned)
            .collect(),
    })
}

pub(crate) fn from_text(text: &str) -> Option<Certificate> {
    if text.len() > 256 * 1024 {
        return None;
    }
    let text = text.trim();
    if text.contains("-----BEGIN CERTIFICATE-----") {
        // First certificate only: a PEM chain is ordered leaf first.
        let der = CertificateDer::from_pem_slice(text.as_bytes()).ok()?;
        from_der(der.as_ref())
    } else {
        let raw: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        from_der(&STANDARD.decode(raw).ok()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const DER: &[u8] = include_bytes!("../test-fixtures/tls/leaf.der");
    const PEM: &str = include_str!("../test-fixtures/tls/leaf.pem");
    #[test]
    fn der_pem_base64_and_pem_chain_share_the_openssl_leaf_fingerprint() {
        let expected = include_str!("../test-fixtures/tls/leaf.sha256").trim();
        for certificate in [
            from_der(DER),
            from_text(PEM),
            from_text(&STANDARD.encode(DER)),
            from_text(&format!("{PEM}\n{PEM}")),
        ] {
            let certificate = certificate.unwrap();
            assert_eq!(certificate.sha256, expected);
            assert_eq!(STANDARD.decode(certificate.der_base64).unwrap(), DER);
            assert!(certificate
                .dns_names
                .contains(&"fixture.example.test".to_string()));
        }
        assert_ne!(format!("{:x}", Sha256::digest(PEM.as_bytes())), expected);
    }
    #[test]
    fn malformed_oversized_and_non_certificate_inputs_never_make_fingerprints() {
        for raw in [
            b"certificate serial 1234".as_slice(),
            &vec![0; 65537],
            &[0x30, 0x01, 0],
        ] {
            assert!(from_der(raw).is_none());
        }
        for raw in [
            "example.com",
            "aabbcc",
            "-----BEGIN CERTIFICATE-----\naaaa\n-----END CERTIFICATE-----",
        ] {
            assert!(from_text(raw).is_none());
        }
        let mut trailing = DER.to_vec();
        trailing.push(0);
        assert!(from_der(&trailing).is_none());
    }
}
