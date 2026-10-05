//! §13 COR — one row per thing that went wrong, with a code the UI and the tests can
//! assert on. A diagnostic never upgrades a claim: a corrupt artifact can only ever
//! reduce what is imported (COR-010).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    #[default]
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub source_path: String,
    pub record_pointer: String,
    pub detail: String,
}

impl Diagnostic {
    pub fn build(
        code: &str,
        severity: Severity,
        source_path: impl Into<String>,
        record_pointer: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code: code.to_string(),
            severity,
            source_path: source_path.into(),
            record_pointer: record_pointer.into(),
            detail: detail.into(),
        }
    }

    pub fn info(code: &str, source_path: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::build(code, Severity::Info, source_path, String::new(), detail)
    }

    pub fn warning(code: &str, source_path: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::build(code, Severity::Warning, source_path, String::new(), detail)
    }

    pub fn error(code: &str, source_path: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::build(code, Severity::Error, source_path, String::new(), detail)
    }

    pub fn at_pointer(
        code: &str,
        severity: Severity,
        source_path: impl Into<String>,
        record_pointer: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::build(code, severity, source_path, record_pointer, detail)
    }
}

/// Sorts diagnostics so a test can compare them without depending on walk order.
pub fn sorted(mut rows: Vec<Diagnostic>) -> Vec<Diagnostic> {
    rows.sort_by(|left, right| {
        (&left.code, &left.source_path, &left.record_pointer).cmp(&(
            &right.code,
            &right.source_path,
            &right.record_pointer,
        ))
    });
    rows
}
