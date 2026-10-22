//! Certificate identity is SHA-256 of the complete leaf X.509 DER, never its
//! subject, serial, public key, PEM text, HTTP body or certificate chain.
use base64::{engine::general_purpose::STANDARD, Engine};
use rustls::pki_types::{pem::PemObject, CertificateDer};
use sha2::{Digest, Sha256};

pub(crate) struct Certificate {
    pub sha256: String,
    pub der_base64: String,
    pub dns_names: Vec<String>,
    pub common_names: Vec<String>,
    pub organizations: Vec<String>,
}

// Read only Subject RDNs from an already validated leaf DER. Issuer O is never
// an ownership hint. Length/tag failures yield no hint, not a guessed identity.
fn subject_fields(der: &[u8]) -> (Vec<String>, Vec<String>) {
    fn next<'a>(bytes: &mut &'a [u8]) -> Option<(u8, &'a [u8])> {
        let tag = *bytes.first()?;
        let first = *bytes.get(1)?;
        let mut offset = 2;
        let len = if first < 128 {
            first as usize
        } else {
            let count = (first & 127) as usize;
            if count == 0 || count > 4 {
                return None;
            }
            let mut len = 0usize;
            for _ in 0..count {
                len = len
                    .checked_mul(256)?
                    .checked_add(*bytes.get(offset)? as usize)?;
                offset += 1;
            }
            len
        };
        let end = offset.checked_add(len)?;
        let value = bytes.get(offset..end)?;
        *bytes = bytes.get(end..)?;
        Some((tag, value))
    }
    fn read(der: &[u8]) -> Option<(Vec<String>, Vec<String>)> {
        let mut outer = der;
        let (tag, mut certificate) = next(&mut outer)?;
        if tag != 0x30 {
            return None;
        }
        let (tag, mut tbs) = next(&mut certificate)?;
        if tag != 0x30 {
            return None;
        }
        if tbs.first() == Some(&0xa0) {
            next(&mut tbs)?;
        }
        for _ in 0..4 {
            next(&mut tbs)?;
        }
        let (tag, mut subject) = next(&mut tbs)?;
        if tag != 0x30 {
            return None;
        }
        let (mut cn, mut org) = (Vec::new(), Vec::new());
        while !subject.is_empty() {
            let (tag, mut set) = next(&mut subject)?;
            if tag != 0x31 {
                return None;
            }
            while !set.is_empty() {
                let (tag, mut pair) = next(&mut set)?;
                if tag != 0x30 {
                    return None;
                }
                let (tag, oid) = next(&mut pair)?;
                if tag != 6 {
                    return None;
                }
                let (tag, raw) = next(&mut pair)?;
                let value = match tag {
                    0x0c | 0x13 | 0x14 | 0x16 => std::str::from_utf8(raw).ok()?.to_string(),
                    0x1e if raw.len() % 2 == 0 => String::from_utf16(
                        &raw.chunks_exact(2)
                            .map(|b| u16::from_be_bytes([b[0], b[1]]))
                            .collect::<Vec<_>>(),
                    )
                    .ok()?,
                    _ => continue,
                };
                let value = value.trim();
                if value.is_empty() || value.chars().any(char::is_control) {
                    continue;
                }
                let out = if oid == [0x55, 4, 3] {
                    &mut cn
                } else if oid == [0x55, 4, 10] {
                    &mut org
                } else {
                    continue;
                };
                if out.len() < 16 {
                    out.push(value.chars().take(240).collect());
                }
            }
        }
        Some((cn, org))
    }
    read(der).unwrap_or_default()
}

pub(crate) fn from_der(der: &[u8]) -> Option<Certificate> {
    if der.is_empty() || der.len() > 64 * 1024 {
        return None;
    }
    let bytes = CertificateDer::from(der);
    let parsed = webpki::EndEntityCert::try_from(&bytes).ok()?;
    let (common_names, organizations) = subject_fields(der);
    Some(Certificate {
        sha256: format!("{:x}", Sha256::digest(der)),
        der_base64: STANDARD.encode(der),
        dns_names: parsed
            .valid_dns_names()
            .map(str::to_owned)
            .collect(),
        common_names,
        organizations,
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

#[cfg(test)]
mod subject_tests {
    use super::*;
    #[test]
    fn organization_is_read_from_subject_and_never_from_issuer() {
        let cert = from_der(include_bytes!(
            "../test-fixtures/tls/subject-organization.der"
        ))
        .unwrap();
        assert_eq!(cert.organizations, vec!["Synthetic Mobile Subsidiary"]);
        assert_eq!(cert.common_names, vec!["owned.example.test"]);
        assert!(cert.dns_names.contains(&"owned.example.test".to_string()));
        assert!(!cert.organizations.iter().any(|v| v.contains("Issuer")));
    }
    #[test]
    fn subject_parser_reads_the_leaf_common_name_without_guessing_issuer_or_organization() {
        let cert = from_der(include_bytes!("../test-fixtures/tls/leaf.der")).unwrap();
        assert_eq!(cert.common_names, vec!["localhost"]);
        assert!(cert.organizations.is_empty());
        assert_eq!(subject_fields(b"not DER"), (vec![], vec![]));
    }
}
