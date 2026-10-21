//! Explicit operator model-connectivity check; resolving a model never invokes it.
use crate::{
    model_configuration::openai_chat_completion_model,
    models::{ModelProfileTestInput, ModelProfileTestResult},
    AppState,
};
use serde_json::Value as JsonValue;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub fn test_model_profile(
    _state: State<AppState>,
    input: ModelProfileTestInput,
) -> Result<ModelProfileTestResult, String> {
    test_model_connection(input)
}

fn test_model_connection(input: ModelProfileTestInput) -> Result<ModelProfileTestResult, String> {
    let deployment = match input.deployment.trim() {
        "local" => "local",
        "cloud" => "cloud",
        _ => return Err("模型部署类型必须是 cloud 或 local".into()),
    };
    let model = input.llm.trim().to_string();
    if model.is_empty() {
        return Err("请填写模型名（例如 openai/模型名）".into());
    }
    let base = input.api_base.trim().trim_end_matches('/').to_string();
    if base.is_empty()
        || !(base.starts_with("http://") || base.starts_with("https://"))
        || base.chars().any(char::is_control)
    {
        return Err("请填写有效的 Base URL（例如 https://api.example.com/v1）".into());
    }
    let api_key = if deployment == "local" {
        let configured = input.api_key.trim();
        if configured.is_empty() {
            "local".to_string()
        } else {
            configured.to_string()
        }
    } else {
        input.api_key.trim().to_string()
    };
    if api_key.is_empty() {
        return Err("云端模型测试必须填写此连接的 API Key；不会使用聊天 OAuth 或环境变量中的凭据".into());
    }
    let chat_model = openai_chat_completion_model(&model);
    if chat_model.is_empty() {
        return Err("模型名不能为空".into());
    }
    let endpoint = format!("{base}/chat/completions");
    let temp_id = Uuid::new_v4();
    let temp_dir = std::env::temp_dir();
    let header_path = temp_dir.join(format!("oviraptor-llm-test-{temp_id}.headers"));
    let body_path = temp_dir.join(format!("oviraptor-llm-test-{temp_id}.json"));
    let response_path = temp_dir.join(format!("oviraptor-llm-test-{temp_id}.response"));
    write_private_temp_file(
        &header_path,
        format!("Authorization: Bearer {api_key}\nContent-Type: application/json\n").as_bytes(),
    )
    .map_err(|error| format!("无法创建模型测试 Header：{error}"))?;
    // §8.2: a chat answer is not enough. The native agent works only through tool
    // calls, so the connectivity test must declare one and see whether the endpoint
    // refuses the schema outright.
    let request_body = serde_json::to_vec(&serde_json::json!({
        "model": chat_model,
        "messages": [{"role": "user", "content": "Use the ok_probe tool with value OK."}],
        "temperature": 0,
        "max_tokens": 64,
        "stream": false,
        "tools": [{
            "type": "function",
            "function": {
                "name": "ok_probe",
                "description": "Report that this endpoint accepts tool calls.",
                "parameters": {
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"],
                    "additionalProperties": false
                }
            }
        }],
        "tool_choice": "auto"
    }))
    .map_err(|error| error.to_string())?;
    if let Err(error) = write_private_temp_file(&body_path, &request_body) {
        let _ = fs::remove_file(&header_path);
        return Err(format!("无法创建模型测试请求：{error}"));
    }
    let output_result = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--location")
        .arg("--connect-timeout")
        .arg("5")
        .arg("--max-time")
        .arg("30")
        .arg("--request")
        .arg("POST")
        .arg("--output")
        .arg(&response_path)
        .arg("--write-out")
        .arg("%{http_code}")
        .arg("-H")
        .arg(format!("@{}", header_path.to_string_lossy()))
        .arg("--data-binary")
        .arg(format!("@{}", body_path.to_string_lossy()))
        .arg(endpoint)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let response_body = fs::read_to_string(&response_path).unwrap_or_default();
    let _ = fs::remove_file(&header_path);
    let _ = fs::remove_file(&body_path);
    let _ = fs::remove_file(&response_path);
    let output = output_result.map_err(|error| format!("无法启动 curl：{error}"))?;
    let code_text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let code = code_text.parse::<u16>().unwrap_or(0);
    if crate::agent_runtime::model::gateway::looks_like_unsupported_tools(code, &response_body) {
        // Reported as a refusal rather than a generic failure: `auto` and native
        // both need this answered before any target is touched (§7.2).
        return Err(format!(
            "模型端点不支持工具调用（HTTP {code}），无法用于原生 Agent；请改用支持 tools 的端点或显式选择兼容后端"
        ));
    }
    if (200..300).contains(&code) {
        let tool_note = if response_body.contains("tool_calls") {
            "，已收到工具调用"
        } else {
            "（端点接受工具声明，本轮未返回工具调用）"
        };
        Ok(ModelProfileTestResult {
            ok: true,
            status: code.to_string(),
            message: format!("模型 Chat Completions 与工具调用测试通过（HTTP {code}）{tool_note}"),
            model,
            deployment: deployment.into(),
        })
    } else {
        let body_error = llm_test_error_message(&response_body);
        let stderr_error = String::from_utf8_lossy(&output.stderr)
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(220)
            .collect::<String>();
        let error_text = if !body_error.is_empty() {
            body_error
        } else if !stderr_error.is_empty() {
            stderr_error
        } else {
            "服务未返回成功响应".into()
        };
        Err(if code > 0 {
            format!("模型 Chat Completions 测试失败（HTTP {code}）：{error_text}")
        } else if error_text.is_empty() {
            "模型连通性测试失败，请检查地址、网络和服务状态".into()
        } else {
            format!("模型连通性测试失败：{error_text}")
        })
    }
}

fn write_private_temp_file(path: &Path, content: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| error.to_string())?;
        file.write_all(content).map_err(|error| error.to_string())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, content).map_err(|error| error.to_string())
    }
}

fn llm_test_error_message(response_body: &str) -> String {
    let trimmed = response_body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let parsed = serde_json::from_str::<JsonValue>(trimmed).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(JsonValue::as_str)
        })
        .unwrap_or(trimmed);
    message
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(260)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_model_test_requires_explicit_endpoint_and_cloud_credential() {
        for deployment in ["cloud", "local"] {
            let missing_endpoint = ModelProfileTestInput {
                llm: "openai/synthetic-model".into(), deployment: deployment.into(),
                api_base: String::new(), api_key: "synthetic-key".into(),
            };
            assert!(test_model_connection(missing_endpoint).err().unwrap().contains("Base URL"));
        }
        let missing_cloud_key = ModelProfileTestInput {
            llm: "openai/synthetic-model".into(), deployment: "cloud".into(),
            api_base: "http://127.0.0.1:9/v1".into(), api_key: String::new(),
        };
        assert!(test_model_connection(missing_cloud_key).err().unwrap().contains("此连接的 API Key"));
    }
}
