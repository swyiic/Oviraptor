use super::*;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};

fn fixture() -> (SignedToolManifest, ToolTrustAnchor, Vec<u8>, Ed25519KeyPair) {
    let package = b"opaque signed worker package".to_vec();
    let digest = Sha256::digest(&package);
    let keypair = Ed25519KeyPair::from_seed_unchecked(&[7u8; 32]).unwrap();
    let manifest = SignedToolManifest {
        payload: ToolCapabilityManifest {
            schema_version: 1,
            id: "browser-runtime".into(),
            version: "123.4".into(),
            platform: "macos-aarch64".into(),
            source: "internal-approved-release".into(),
            digest: format!("sha256:{digest:x}"),
            license: "recorded-license".into(),
            entrypoint: "bin/browser".into(),
            broker_capabilities: vec!["browser_action".into()],
            network_policy: "broker-only".into(),
            filesystem_policy: "read-only-input-and-private-temp-output".into(),
            timeout_seconds: 150,
            memory_mib: 768,
            cpu_limit: 1,
            process_limit: 32,
            output_bytes: 2_097_152,
            input_schema_version: 1,
            output_schema_version: 1,
        },
        signer_id: "release-key-1".into(),
        signature_base64: String::new(),
    };
    let anchor = ToolTrustAnchor {
        signer_id: manifest.signer_id.clone(),
        public_key: keypair.public_key().as_ref().to_vec(),
        revoked: false,
    };
    (manifest, anchor, package, keypair)
}

fn sign(manifest: &mut SignedToolManifest, keypair: &Ed25519KeyPair) -> Vec<u8> {
    manifest.signature_base64 = base64::engine::general_purpose::STANDARD.encode(
        keypair
            .sign(&signed_message(&manifest.payload).unwrap())
            .as_ref(),
    );
    serde_json::to_vec(manifest).unwrap()
}

#[test]
fn only_a_trusted_signed_exact_platform_and_package_digest_is_accepted() {
    let (mut manifest, anchor, package, keypair) = fixture();
    let raw = sign(&mut manifest, &keypair);
    assert_eq!(
        verify_staged_tool_blob(
            &raw,
            &package,
            &[ToolTrustAnchor {
                signer_id: anchor.signer_id.clone(),
                public_key: anchor.public_key.clone(),
                revoked: false,
            }],
            "macos-aarch64"
        )
        .unwrap(),
        manifest.payload
    );
    assert_eq!(
        verify_staged_tool_blob(&raw, b"changed", &[anchor], "macos-aarch64").unwrap_err(),
        "tool_package_digest_mismatch"
    );
    assert_eq!(
        verify_staged_tool_blob(&raw, &package, &[], "macos-aarch64").unwrap_err(),
        "tool_signer_not_trusted_or_revoked"
    );
}

#[test]
fn revocation_tampering_unknown_fields_and_cross_platform_packages_fail_closed() {
    let (mut manifest, mut anchor, package, keypair) = fixture();
    let raw = sign(&mut manifest, &keypair);
    assert_eq!(
        verify_staged_tool_blob(
            &raw,
            &package,
            &[ToolTrustAnchor {
                signer_id: anchor.signer_id.clone(),
                public_key: anchor.public_key.clone(),
                revoked: false,
            }],
            "linux-x86_64"
        )
        .unwrap_err(),
        "tool_manifest_policy_invalid"
    );
    anchor.revoked = true;
    assert_eq!(
        verify_staged_tool_blob(&raw, &package, &[anchor], "macos-aarch64").unwrap_err(),
        "tool_signer_not_trusted_or_revoked"
    );
    manifest.payload.network_policy = "host".into();
    let tampered = serde_json::to_vec(&manifest).unwrap();
    assert_eq!(
        verify_staged_tool_blob(&tampered, &package, &[], "macos-aarch64").unwrap_err(),
        "tool_manifest_policy_invalid"
    );
    let injected = String::from_utf8(raw)
        .unwrap()
        .replace("\"signerId\":", "\"extra\":true,\"signerId\":");
    assert_eq!(
        verify_staged_tool_blob(injected.as_bytes(), &package, &[], "macos-aarch64").unwrap_err(),
        "tool_manifest_invalid_json"
    );
}

#[test]
fn a_signature_cannot_authorize_a_different_policy_or_path() {
    let (mut manifest, anchor, package, keypair) = fixture();
    let _ = sign(&mut manifest, &keypair);
    manifest.payload.entrypoint = "bin/other".into();
    let raw = serde_json::to_vec(&manifest).unwrap();
    assert_eq!(
        verify_staged_tool_blob(&raw, &package, &[anchor], "macos-aarch64").unwrap_err(),
        "tool_signature_invalid"
    );
    for invalid in [
        "/bin/browser",
        "../browser",
        "bin/../browser",
        "bin//browser",
        "C:\\browser",
        "bin\\browser",
        "bin/\0browser",
    ] {
        assert!(!valid_entrypoint(invalid), "{invalid}");
    }
}

#[test]
fn package_policy_does_not_let_browser_go_direct_or_a_source_parser_use_network() {
    let (mut manifest, anchor, package, keypair) = fixture();
    manifest.payload.network_policy = "none".into();
    let raw = sign(&mut manifest, &keypair);
    assert_eq!(
        verify_staged_tool_blob(&raw, &package, &[anchor], "macos-aarch64").unwrap_err(),
        "tool_manifest_policy_invalid"
    );
    manifest.payload.broker_capabilities = vec!["source_analysis".into()];
    let raw = sign(&mut manifest, &keypair);
    let anchor = ToolTrustAnchor {
        signer_id: manifest.signer_id.clone(),
        public_key: keypair.public_key().as_ref().to_vec(),
        revoked: false,
    };
    assert!(verify_staged_tool_blob(&raw, &package, &[anchor], "macos-aarch64").is_ok());
}

#[path = "tests/staging.rs"]
mod staging;
