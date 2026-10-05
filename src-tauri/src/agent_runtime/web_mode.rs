//! Strict new ordinary-Web mode; no policy default is runtime authority.
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) mod action;
pub(crate) mod draft;
pub(crate) mod root;
mod finance;
pub(crate) mod writer;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum WebMode {
    Single,
    Multi,
}
impl WebMode {
    pub(crate) fn new_input(value: Option<&str>) -> Result<Self, String> {
        match value {
            None | Some("multi") => Ok(Self::Multi),
            Some("single") => Ok(Self::Single),
            _ => Err("web_mode_invalid".into()),
        }
    }
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Multi => "multi",
        }
    }
    pub(crate) fn from_policy(policy: &Value) -> Result<Self, String> {
        let v = policy.get("orchestration").ok_or("web_mode_missing")?;
        if v.as_object().is_none_or(|o| o.len() != 2) || v["version"] != 1 {
            return Err("web_mode_invalid".into());
        }
        match v["mode"].as_str() {
            Some("single") => Ok(Self::Single),
            Some("multi") => Ok(Self::Multi),
            _ => Err("web_mode_invalid".into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModeFact {
    pub schema_version: u8,
    pub receipt_id: String,
    pub draft_id: String,
    pub scan_id: String,
    pub attempt_number: i64,
    pub execution_mode: String,
    pub parent_receipt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_action_hash: Option<String>,
    pub mode: WebMode,
    pub work_dir: String,
    pub task_hash: String,
    pub policy_hash: String,
    pub runtime_hash: String,
    pub startup_tag: String,
    pub targets: Vec<String>,
}
impl ModeFact {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.attempt_number < 1
            || self.scan_id.is_empty()
            || self.work_dir.is_empty()
            || [self.receipt_id.as_str(), self.draft_id.as_str()]
                .iter()
                .any(|s| uuid::Uuid::parse_str(s).is_err())
            || !matches!(self.execution_mode.as_str(), "initial" | "fresh" | "resume")
            || (self.execution_mode == "resume") != self.parent_receipt_id.is_some()
            || self
                .parent_receipt_id
                .as_ref()
                .is_some_and(|s| uuid::Uuid::parse_str(s).is_err())
            || self
                .creation_action_hash
                .as_ref()
                .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            || [
                &self.task_hash,
                &self.policy_hash,
                &self.runtime_hash,
                &self.startup_tag,
            ]
            .iter()
            .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            || self.targets.is_empty()
            || self.targets.len() > 200
            || self
                .targets
                .iter()
                .any(|s| !(s.starts_with("http://") || s.starts_with("https://")))
            || self.targets.windows(2).any(|s| s[0] >= s[1])
        {
            return Err("web_mode_receipt_invalid".into());
        }
        Ok(())
    }
    pub(crate) fn message(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(&serde_json::json!({"domain":"oviraptor.web-mode.v1","fact":self}))
            .map_err(|e| e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerifiedMode {
    fact: ModeFact,
}
impl VerifiedMode {
    pub(crate) fn fact(&self) -> &ModeFact {
        &self.fact
    }
    pub(crate) fn verify(fact: ModeFact, key: &[u8], signature: &[u8]) -> Result<Self, String> {
        if key.len() != 32 || signature.len() != 32 {
            return Err("web_mode_private_proof_invalid".into());
        }
        aws_lc_rs::hmac::verify(
            &aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, key),
            &fact.message()?,
            signature,
        )
        .map_err(|_| "web_mode_private_proof_changed")?;
        Ok(Self { fact })
    }
}

pub(crate) fn stored_fact(
    db: &Connection,
    scan: &str,
    attempt: i64,
) -> Result<(ModeFact, Vec<u8>), String> {
    let (text, signature): (String, Vec<u8>) = db
        .query_row(
            "SELECT fact_json,signature FROM native_web_mode_receipts
        WHERE scan_id=?1 AND attempt_number=?2",
            rusqlite::params![scan, attempt],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "web_mode_receipt_missing")?;
    let fact: ModeFact = serde_json::from_str(&text).map_err(|_| "web_mode_receipt_invalid")?;
    if serde_json::to_string(&fact).map_err(|e| e.to_string())? != text
        || fact.scan_id != scan
        || fact.attempt_number != attempt
    {
        return Err("web_mode_receipt_changed".into());
    }
    fact.validate()?;
    Ok((fact, signature))
}
