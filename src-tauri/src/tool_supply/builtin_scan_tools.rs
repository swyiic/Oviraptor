//! Built-in scan workers that already ship with the desktop binary.
//!
//! The approved-package sandbox runner stays closed until hard per-run memory
//! containment is verified. These workers are the scan tools that exist today;
//! they are not nmap, nuclei, or an open sandbox execution API.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BuiltinScanTool {
    pub id: &'static str,
    pub worker: Option<&'static str>,
    pub network: &'static str,
    pub sandbox: &'static str,
    pub executes: bool,
}

pub(crate) fn catalog() -> &'static [BuiltinScanTool] {
    &[
        BuiltinScanTool {
            id: "http_recon",
            worker: None,
            network: "direct-http-no-broker",
            sandbox: "none",
            executes: true,
        },
        BuiltinScanTool {
            id: "js_ast_analyzer",
            worker: Some("8_js_ast_analyzer.cjs"),
            network: "none",
            sandbox: "process-timeout-only",
            executes: true,
        },
        BuiltinScanTool {
            id: "frontend_runtime_probe",
            worker: Some("9_frontend_runtime_probe.cjs"),
            network: "browser-fetch-gate",
            sandbox: "fetch-intercept-not-rust-broker",
            executes: false,
        },
    ]
}

pub(crate) const SANDBOX_RUNNER: &str = "closed:macos_hard_memory_containment_unverified";

pub(crate) fn summary() -> String {
    let running = catalog().iter().filter(|tool| tool.executes).map(|tool| tool.id).collect::<Vec<_>>().join(", ");
    let held = catalog().iter().filter(|tool| !tool.executes).map(|tool| tool.id).collect::<Vec<_>>().join(", ");
    format!("可执行 {running}。已随包提供但不可执行 {held}。批准包沙箱执行器仍关闭：{SANDBOX_RUNNER}")
}

