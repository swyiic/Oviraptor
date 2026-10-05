// Product-level Agent entry points that do not belong to a more specific module.
#[tauri::command]
pub fn test_model_profile(
    _state: State<AppState>,
    input: ModelProfileTestInput,
) -> Result<ModelProfileTestResult, String> {
    test_model_connection(input)
}
