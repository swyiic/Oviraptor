// Static regression guard, not a desktop IPC integration test.
#[test]
fn implicit_history_import_commands_are_not_registered() {
    let registrations = include_str!("../lib.rs");
    let modules = include_str!("../commands.rs");
    for command in ["sync_sentinel_results", "run_artifact_import"] {
        assert!(
            !registrations.contains(&format!("commands::{command}")),
            "retired implicit importer is registered: {command}"
        );
    }
    assert!(!modules.contains("commands/result_ingestion_sync.rs"));
    assert!(!modules.contains("commands/legacy_artifact_compat.rs"));
    let ingestion = include_str!("result_ingestion_runs.rs");
    for retired in [
        "historical_artifact_roots",
        "legacyArtifactDirectories",
        "expand_home_path",
        "strix_run_dirs",
        "LEGACY_RUN_ARTIFACT",
        "legacy_completed_artifact",
    ] {
        assert!(
            !ingestion.contains(retired),
            "implicit root discovery survived: {retired}"
        );
    }
}
