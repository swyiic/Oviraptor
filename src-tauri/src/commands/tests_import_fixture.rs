// Explicit fixture roots only; no application command discovers historical directories.
fn artifact_import_report(
    connection: &rusqlite::Connection,
    cas_dir: &Path,
    key_path: &Path,
    roots: &[PathBuf],
) -> Result<ArtifactImportReport, String> {
    let limits = crate::artifact_import::Limits::default();
    let context = crate::artifact_import::ImportContext {
        connection,
        cas_dir,
        key_path,
        roots,
        limits: &limits,
    };
    let summary = crate::artifact_import::import_roots(&context);
    Ok(ArtifactImportReport::build(&summary, connection))
}
