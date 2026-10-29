//! Shared model configuration and gateway-profile resolution.
//! Reads the caller's existing snapshot; no scan, tool grant or runtime mutation
//! is started by resolving a profile.
use crate::{
    agent_runtime::model::{GatewayProfile, LocalResourcePolicy, ResolvedModelProfile},
    db,
};
use serde_json::Value as JsonValue;
use std::process::Command;

pub(crate) fn default_settings(connection: &rusqlite::Connection) -> JsonValue {
    crate::configuration_catalog::read_current_defaults(connection).unwrap_or_else(|_| {
        serde_json::json!({"_configurationError":"当前分类配置无法读取，请重新打开设置检查；未使用旧策略或发送模型请求"})
    })
}

/// A runtime uses only the configuration classes it consumes.
pub(crate) fn default_settings_for(connection: &rusqlite::Connection, kinds: &[&str]) -> JsonValue {
    crate::configuration_catalog::read_current_defaults_for(connection, kinds).unwrap_or_else(|_| {
        serde_json::json!({"_configurationError":"当前分类配置无法读取，请重新打开设置检查；未使用旧策略或发送模型请求"})
    })
}

/// Normalize stored aliases once, before either connection selection or a
/// caller's domain-specific settings are inspected. This carries no authority.
pub(crate) struct NormalizedModelSettings(JsonValue);

impl NormalizedModelSettings {
    pub(crate) fn new(settings: &JsonValue) -> Self {
        Self(db::normalize_settings(settings))
    }
    pub(crate) fn as_value(&self) -> &JsonValue {
        &self.0
    }
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct ModelConnection {
    pub(crate) llm: String,
    pub(crate) api_key: String,
    pub(crate) api_base: String,
    pub(crate) deployment: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalModelResourcePolicy {
    pub(crate) parameter_billions: Option<u32>,
    pub(crate) unified_memory_gb: u64,
    pub(crate) max_concurrent_requests: usize,
    pub(crate) max_context_tokens: u64,
    pub(crate) max_output_tokens: Option<u64>,
    pub(crate) memory_guard_tier: &'static str,
}

pub(crate) fn model_parameter_billions(model: &str) -> Option<u32> {
    model
        .to_ascii_lowercase()
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '.'))
        .filter_map(|token| token.strip_suffix('b'))
        .filter_map(|number| number.parse::<f64>().ok())
        .filter(|number| *number >= 1.0 && *number <= 1000.0)
        .map(f64::ceil)
        .map(|number| number as u32)
        .max()
}

pub(crate) fn system_unified_memory_gb() -> u64 {
    if cfg!(target_os = "macos") {
        if let Ok(output) = Command::new("sysctl").args(["-n", "hw.memsize"]).output() {
            if output.status.success() {
                if let Ok(bytes) = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<u64>()
                {
                    return ((bytes + (1 << 30) - 1) >> 30).max(1);
                }
            }
        }
    }
    32
}

pub(crate) fn model_resource_policy_for_memory(
    environment: &ModelConnection,
    unified_memory_gb: u64,
) -> LocalModelResourcePolicy {
    if environment.deployment != "local" {
        return LocalModelResourcePolicy {
            parameter_billions: None,
            unified_memory_gb,
            max_concurrent_requests: 4,
            max_context_tokens: 0,
            max_output_tokens: None,
            memory_guard_tier: "balanced",
        };
    }
    let parameter_billions = model_parameter_billions(&environment.llm);
    let size = parameter_billions.unwrap_or(20);
    let max_context_tokens = match unified_memory_gb {
        0..=18 if size <= 10 => 49_152,
        0..=18 if size <= 20 => 24_576,
        0..=18 => 16_384,
        19..=24 if size <= 10 => 65_536,
        19..=24 if size <= 20 => 49_152,
        19..=24 => 32_768,
        25..=32 if size <= 10 => 65_536,
        25..=32 if size <= 20 => 57_344,
        25..=32 if size <= 39 => 49_152,
        25..=32 => 32_768,
        33..=48 if size <= 10 => 98_304,
        33..=48 if size <= 20 => 65_536,
        33..=48 if size <= 39 => 57_344,
        33..=48 => 49_152,
        _ if size <= 10 => 131_072,
        _ if size <= 20 => 98_304,
        _ if size <= 39 => 65_536,
        _ if size <= 70 => 49_152,
        _ => 32_768,
    };
    let max_output_tokens = match parameter_billions {
        Some(size) if size >= 60 => 3_072,
        Some(size) if size >= 20 => 3_072,
        Some(size) if size >= 13 => 3_072,
        Some(_) => 2_048,
        None => 3_072,
    };
    let memory_guard_tier = if unified_memory_gb <= 18 && size <= 10 {
        // A 9B Q4 model's first prefill sits just above oMLX Balanced on a
        // 16 GB Mac. Aggressive still remains below the
        // physical Metal ceiling and keeps the guard enabled.
        "aggressive"
    } else {
        "balanced"
    };
    LocalModelResourcePolicy {
        parameter_billions,
        unified_memory_gb,
        // Keep local prefills serial under the existing memory policy.
        max_concurrent_requests: 1,
        max_context_tokens,
        max_output_tokens: Some(max_output_tokens),
        memory_guard_tier,
    }
}

pub(crate) fn model_resource_policy(environment: &ModelConnection) -> LocalModelResourcePolicy {
    model_resource_policy_for_memory(environment, system_unified_memory_gb())
}

fn profile_with_id<'a>(
    profiles: Option<&'a JsonValue>,
    active_id: Option<&JsonValue>,
) -> Option<&'a JsonValue> {
    let profiles = profiles?.as_array()?;
    if profiles.is_empty() {
        return None;
    }
    let active_id = active_id
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    profiles
        .iter()
        .find(|profile| {
            !active_id.is_empty()
                && profile.get("id").and_then(JsonValue::as_str) == Some(active_id.as_str())
        })
        .or_else(|| profiles.first())
}

/// Settings aliases are normalized at the migration boundary, not selected by
/// the model runtime. Only the neutral profile list is authoritative here.
fn active_model_profile(settings: &JsonValue) -> Option<&JsonValue> {
    profile_with_id(
        settings.get("modelProfiles"),
        settings.get("activeModelProfileId"),
    )
}

pub(crate) fn model_connection(
    settings: &NormalizedModelSettings,
) -> Result<ModelConnection, String> {
    model_connection_with(settings, &crate::secret_store::SecretStore::system())
}

fn model_connection_with(
    settings: &NormalizedModelSettings,
    secrets: &crate::secret_store::SecretStore,
) -> Result<ModelConnection, String> {
    let settings = settings.as_value();
    if let Some(error) = settings
        .get("_configurationError")
        .and_then(JsonValue::as_str)
    {
        return Err(error.to_string());
    }
    let active_profile = active_model_profile(settings);
    let independently_owned = settings
        .get("_configurationRefs")
        .and_then(|refs| refs.get("models"))
        .is_some();
    if independently_owned
        && active_profile
            .is_some_and(|profile| profile.get("id") != settings.get("activeModelProfileId"))
    {
        return Err("独立模型的当前选择无效，请重新打开模型设置".into());
    }
    if settings.get("modelProfiles").is_some() && active_profile.is_none() {
        return Err("模型未配置：模型列表为空或无效，请在配置中心选择模型后再启动".into());
    }
    // Once a profile is selected it owns the whole endpoint configuration.
    // Empty/missing fields must not borrow a previous profile's flat cache.
    let profile_value = |key: &str, neutral_key: &str| match active_profile {
        Some(profile) => profile.get(key),
        None => settings.get(neutral_key),
    };
    // A profile is normal, but a settings blob that only carries the flat neutral
    // keys must still be able to say which side it belongs to (§8.1).
    let deployment = profile_value("deployment", "modelDeployment")
        .and_then(JsonValue::as_str)
        .filter(|value| *value == "local")
        .unwrap_or("cloud")
        .to_string();
    let profile_setting = |key: &str, neutral_key: &str| {
        profile_value(key, neutral_key)
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let llm = profile_setting("llm", "modelName");
    if llm.is_empty() {
        return Err(
            "模型未配置：请在配置中心填写当前模型名称和 API 地址后再启动。任务尚未发出任何请求"
                .into(),
        );
    }
    let api_key = if deployment == "local" {
        // Keep local authentication separate from a profile's cloud key. This
        // avoids sending a hidden stale cloud credential to localhost while
        // still supporting self-hosted gateways that require their own token.
        // Neither the cloud key nor the environment fallback below may reach a
        // local endpoint, in either key spelling (§8.1).
        profile_value("localApiKey", "localApiKey")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("local")
            .to_string()
    } else {
        let configured = profile_setting("apiKey", "modelApiKey");
        if !configured.is_empty() {
            configured
        } else if independently_owned {
            String::new()
        } else {
            std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_default()
        }
    };
    let api_base = if deployment == "local" {
        profile_setting("apiBase", "modelApiBase")
    } else {
        let configured = profile_setting("apiBase", "modelApiBase");
        if !configured.is_empty() {
            configured
        } else if independently_owned {
            "https://api.openai.com/v1".into()
        } else {
            std::env::var("OPENAI_BASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_default()
        }
    };
    if deployment == "local" && api_base.is_empty() {
        return Err(
            "本地模型必须配置明确的 API Base URL；安全策略不会回退读取 OPENAI_BASE_URL，防止误用云端环境变量或凭据".into(),
        );
    }
    // Credential→keychain read-through (docs/KEYCHAIN_MIGRATION_PLAN.md). Default
    // OFF → returns `api_key` unchanged (byte-for-byte today's behaviour). ON → the
    // stored plaintext stays authoritative and is synced into the keychain (so a
    // later phase can blank the plaintext and read from the keychain); scoped by
    // the active model-profile id so profiles never share a key, and a keychain
    // outage just falls back to the plaintext so a credential is never lost.
    let api_key = {
        let scope = active_profile
            .and_then(|profile| profile.get("id"))
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string();
        let id = if deployment == "local" {
            crate::secret_store::llm_local_api_key(&scope)
        } else {
            crate::secret_store::llm_api_key(&scope)
        };
        secrets.resolve(&id, Some(&api_key)).unwrap_or_default()
    };
    if api_key.is_empty() {
        return Err(
            "模型 API 凭据未配置：请在配置中心填写 API Key（第三方 OpenAI 兼容服务同时填写 Base URL）".into(),
        );
    }
    Ok(ModelConnection {
        llm,
        api_key,
        api_base,
        deployment,
    })
}

pub(crate) fn openai_chat_completion_model(llm: &str) -> String {
    let value = llm.trim();
    value
        .strip_prefix("openai/")
        .unwrap_or(value)
        .trim()
        .to_string()
}

pub(crate) fn gateway_profile(
    environment: &ModelConnection,
    proxy: Option<&str>,
) -> Result<GatewayProfile, String> {
    let policy = model_resource_policy(environment);
    GatewayProfile::resolve(
        &ResolvedModelProfile {
            model: openai_chat_completion_model(&environment.llm),
            api_base: environment.api_base.clone(),
            api_key: environment.api_key.clone(),
            deployment: environment.deployment.clone(),
        },
        &LocalResourcePolicy {
            max_output_tokens: policy.max_output_tokens,
            max_context_tokens: policy.max_context_tokens,
        },
        proxy,
    )
}

/// Resolve the application-selected model inside the caller's connection.
/// Callers retain their own endpoint, consent and execution policies.
pub(crate) fn gateway_profile_on(
    connection: &rusqlite::Connection,
    proxy: Option<&str>,
) -> Result<GatewayProfile, String> {
    let settings = NormalizedModelSettings::new(&default_settings_for(connection, &["models"]));
    let environment = model_connection(&settings)?;
    gateway_profile(&environment, proxy)
}


#[cfg(test)]
mod keychain_wiring_tests {
    use super::*;
    use crate::secret_store::{llm_api_key, testing};

    fn cloud_settings(api_key: &str) -> NormalizedModelSettings {
        NormalizedModelSettings::new(&serde_json::json!({
            "modelProfiles": [{
                "id": "mp1",
                "llm": "openai/gpt-4o",
                "deployment": "cloud",
                "apiBase": "https://api.openai.com/v1",
                "apiKey": api_key,
            }],
            "activeModelProfileId": "mp1",
        }))
    }

    #[test]
    fn keychain_off_returns_plaintext_unchanged_and_never_writes() {
        let (store, backend) = testing::store(true, false);
        let connection = model_connection_with(&cloud_settings("sk-legacy"), &store).unwrap();
        assert_eq!(connection.api_key, "sk-legacy");
        assert_eq!(backend.len(), 0, "read-through OFF must not touch the keychain");
    }

    #[test]
    fn keychain_on_syncs_the_key_and_follows_db_rotation() {
        let (store, backend) = testing::store(true, true);
        let connection = model_connection_with(&cloud_settings("sk-legacy"), &store).unwrap();
        assert_eq!(connection.api_key, "sk-legacy");
        assert_eq!(backend.len(), 1, "the key is synced into the keychain scoped by profile");
        // A key rotated in the DB wins and re-syncs the keychain.
        let rotated = model_connection_with(&cloud_settings("sk-rotated"), &store).unwrap();
        assert_eq!(rotated.api_key, "sk-rotated");
    }

    #[test]
    fn keychain_on_but_unavailable_still_resolves_from_plaintext() {
        let (store, backend) = testing::store(false, true);
        let connection = model_connection_with(&cloud_settings("sk-legacy"), &store).unwrap();
        assert_eq!(connection.api_key, "sk-legacy");
        assert_eq!(backend.len(), 0);
    }

    #[test]
    fn scope_is_the_active_model_profile_id() {
        let (store, _backend) = testing::store(true, true);
        let _ = model_connection_with(&cloud_settings("sk-legacy"), &store).unwrap();
        // The synced entry is keyed by the model-profile id, matching the read scope.
        assert_eq!(
            store.resolve(&llm_api_key("mp1"), None),
            Some("sk-legacy".to_string())
        );
    }
}

// Real-machine end-to-end proof that an LLM key sourced through the keychain
// reaches the actual HTTP Authorization header, using the real gateway/transport
// stack against a local capture server and a synthetic DeepSeek-shaped profile.
// #[ignore]d and macOS-only: run with
//   cargo test --lib keychain_e2e -- --ignored --nocapture
#[cfg(all(test, target_os = "macos"))]
mod keychain_e2e_tests {
    use super::*;
    use crate::agent_runtime::model::{CancelToken, ModelRequest, OpenAiCompatibleGateway};
    use crate::secret_store::{llm_api_key, KeychainBackend, SecretBackend, SecretStore};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// A local HTTP server that records every request's Authorization header and
    /// answers with a minimal, well-formed chat completion so the real client's
    /// one-shot call returns promptly (Content-Length + Connection: close).
    fn start_capture_server(auths: Arc<Mutex<Vec<String>>>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let header_end = loop {
                    match stream.read(&mut chunk) {
                        Ok(0) => break buf.len(),
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                break pos + 4;
                            }
                            if buf.len() > 1_000_000 {
                                break buf.len();
                            }
                        }
                        Err(_) => break buf.len(),
                    }
                };
                let head = String::from_utf8_lossy(&buf[..header_end.min(buf.len())]).to_string();
                for line in head.lines() {
                    if line.to_ascii_lowercase().starts_with("authorization:") {
                        let value = line[line.find(':').unwrap() + 1..].trim().to_string();
                        auths.lock().unwrap().push(value);
                    }
                }
                // Drain the request body so closing the socket never RSTs the client.
                let content_length = head
                    .lines()
                    .find_map(|line| {
                        let lower = line.to_ascii_lowercase();
                        lower
                            .strip_prefix("content-length:")
                            .map(|rest| rest.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                let mut body_read = buf.len().saturating_sub(header_end);
                while body_read < content_length {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => body_read += n,
                    }
                }
                let body = br#"{"choices":[{"message":{"role":"assistant","content":"OK"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
                let _ = stream.flush();
            }
        });
        port
    }

    fn settings(base: &str, api_key: &str) -> NormalizedModelSettings {
        NormalizedModelSettings::new(&serde_json::json!({
            // Marks the config as independently-owned so an empty key never falls
            // back to a real OPENAI_API_KEY env var during the blanked-DB phase.
            "_configurationRefs": {"models": {}},
            "modelProfiles": [{
                "id": "mp-e2e",
                "llm": "deepseek-flash",
                "deployment": "cloud",
                "apiBase": base,
                "apiKey": api_key,
            }],
            "activeModelProfileId": "mp-e2e",
        }))
    }

    fn send_once(settings: &NormalizedModelSettings, store: &SecretStore) -> String {
        let environment = model_connection_with(settings, store).unwrap();
        let gateway = gateway_profile(&environment, None).unwrap();
        let client = OpenAiCompatibleGateway::new(gateway, &[]);
        let request =
            ModelRequest::new(vec![serde_json::json!({"role": "user", "content": "ping"})], vec![]);
        let _ = client.complete_once_observed(&request, &CancelToken::new());
        environment.api_key
    }

    #[test]
    #[ignore = "real macOS keychain + local HTTP; run with --ignored"]
    fn key_flows_from_keychain_to_the_authorization_header() {
        const SERVICE: &str = "com.oviraptor.desktop.e2e";
        const KEY: &str = "sk-deepseek-SYNTHETIC-e2e-abcdef0123";
        let id = llm_api_key("mp-e2e");
        let _ = KeychainBackend::new(SERVICE).delete(&id); // clean slate

        let auths = Arc::new(Mutex::new(Vec::new()));
        let port = start_capture_server(auths.clone());
        let base = format!("http://127.0.0.1:{port}/v1");
        let store = SecretStore::new(Some(Box::new(KeychainBackend::new(SERVICE))), true);

        // Phase A: the DB holds the key (flag ON). The request carries it and the
        // key is synced into the keychain.
        let resolved_a = send_once(&settings(&base, KEY), &store);
        assert_eq!(resolved_a, KEY);

        // Phase B: the DB key is blanked. The only remaining source is the keychain,
        // so the request must STILL carry the synthetic key.
        let resolved_b = send_once(&settings(&base, ""), &store);
        assert_eq!(resolved_b, KEY, "key must be served from the keychain when the DB is blank");

        let captured = auths.lock().unwrap().clone();
        KeychainBackend::new(SERVICE).delete(&id).unwrap(); // cleanup

        assert!(captured.len() >= 2, "expected two requests on the wire, got {captured:?}");
        for value in &captured {
            assert_eq!(value, &format!("Bearer {KEY}"), "wire header must carry the key");
        }
    }
}
