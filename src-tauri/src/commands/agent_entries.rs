// Neutral entry points for the product surface (Phase 2 §9).
//
// A native investigation is not a "Strix scan", so the front end calls these
// names. The Strix-named commands stay registered and simply forward here: they are
// the ABI contract with saved front-ends and external callers, and removing them
// would break a working install for the sake of a label.
#[tauri::command]
pub fn test_model_profile(
    state: State<AppState>,
    input: StrixLlmTestInput,
) -> Result<StrixLlmTestResult, String> {
    test_strix_llm(state, input)
}

#[tauri::command]
pub fn start_workbench_scan(
    app: AppHandle,
    state: State<AppState>,
    input: StrixWorkbenchInput,
) -> Result<SentinelScan, String> {
    start_strix_workbench_scan(app, state, input)
}

#[tauri::command]
pub fn rescan_workbench_scan(
    app: AppHandle,
    state: State<AppState>,
    scan_id: String,
) -> Result<SentinelScan, String> {
    rescan_strix_workbench_scan(app, state, scan_id)
}

#[tauri::command]
pub fn list_agent_traces(state: State<AppState>) -> Result<Vec<StrixTraceSummary>, String> {
    list_strix_traces(state)
}

#[tauri::command]
pub fn get_agent_trace(
    state: State<AppState>,
    scan_id: String,
) -> Result<StrixTraceDetail, String> {
    get_strix_trace(state, scan_id)
}

#[tauri::command]
pub fn list_agent_instructions(state: State<AppState>) -> Result<Vec<StrixSkill>, String> {
    list_strix_skills(state)
}

#[tauri::command]
pub fn save_agent_instruction(
    state: State<AppState>,
    input: StrixSkillInput,
) -> Result<i64, String> {
    save_strix_skill(state, input)
}
