use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn commit_bundle(
    context: &ImportContext<'_>,
    root: &Path,
    bundle: &Bundle,
    manifest: &Manifest,
    signature: &str,
    cas_files: &CasRows,
    groups: &[ScopedRecords],
    diagnostics: &[Diagnostic],
) -> Result<BundleCommit, String> {
    let transaction = begin_bundle_transaction(context.connection)?;
    let bundle_row_id = store::write_bundle(
        &transaction,
        &manifest.bundle_id,
        &bundle.source_dir,
        root,
        &bundle.attempt_key,
        groups
            .iter()
            .map(|group| group.scope.attempt_number)
            .max()
            .unwrap_or(0),
        signature,
        BundleStatus::Imported.as_str(),
        groups.iter().map(|group| group.records.len()).sum(),
        0,
    )?;
    // Object rows join the same transaction as the projection they belong to, so a
    // rollback removes them from every query surface at once (缺口 7).
    let mut file_rows = Vec::new();
    for object in cas_files {
        let object_id = store::upsert_object(
            &transaction,
            &object.content_hash,
            object.bytes,
            &object.storage_path,
            &object.display_text,
            object.secret_material,
        )
        .map_err(|error| format!("{}：{error}", object.relative_path))?;
        file_rows.push((
            object.relative_path.clone(),
            object.content_hash.clone(),
            object.bytes,
            object_id,
        ));
    }
    store::replace_bundle_files(&transaction, bundle_row_id, &file_rows)?;
    let mut report = reconcile::ReconcileReport {
        revoked: adapters::source_report::retire_legacy_memberships(&transaction, groups)?,
        ..Default::default()
    };
    let mut extra = Vec::new();
    let mut accepted = Vec::new();
    let mut eligible = false;
    for group in groups {
        let (applied, notes) = reconcile::apply(
            &transaction,
            &group.scope,
            bundle_row_id,
            &bundle.source_dir,
            &group.records,
            &envelope_text,
            context.limits,
        )?;
        if !notes.iter().any(|note| {
            matches!(
                note.code.as_str(),
                "older_attempt_ignored" | "deleted_scan_not_projected"
            )
        }) {
            eligible = true;
            accepted.extend(group.records.values());
        }
        report.revisions += applied.revisions;
        report.revoked += applied.revoked;
        extra.extend(notes);
    }
    for diagnostic in diagnostics.iter().chain(extra.iter()) {
        store::record_diagnostic(
            &transaction,
            Some(bundle_row_id),
            &root.to_string_lossy(),
            diagnostic,
        )?;
    }
    // The stored status is what the read-only view counts, so it is derived by the
    // same rule the outcome reports (缺口 6 must not have two answers).
    let status = status_of(diagnostics, &extra, report.revisions, report.revoked);
    let updated = transaction
        .execute(
            "UPDATE import_bundles SET revision_count=?1,status=?2 WHERE id=?3",
            rusqlite::params![report.revisions as i64, status.as_str(), bundle_row_id],
        )
        .map_err(|error| error.to_string())?;
    if updated != 1 {
        return Err(format!("bundle 行数异常：{updated}"));
    }
    let committed_records = if status == BundleStatus::Failed || !eligible {
        None
    } else {
        let mut receipts = Vec::with_capacity(accepted.len());
        for record in accepted {
            let (revision_id, text): (i64, String) = transaction.query_row(
                "SELECT id,envelope_json FROM import_record_revisions WHERE record_key=?1 AND logical_key=?1 AND revision_hash=?2 AND record_kind=?3",
                rusqlite::params![record.logical_key,record.revision_hash,record.record_kind.as_str()],
                |row|Ok((row.get(0)?,row.get(1)?))).map_err(|error|error.to_string())?;
            let envelope_sha256 = crate::artifact_import::canonical::sha256_hex(text.as_bytes());
            let mut artifacts = record.contributing_artifacts.clone();
            artifacts.insert(record.provenance.source_artifact_id.clone());
            for source_artifact_id in artifacts {
                receipts.push(ImportedRecordReceipt {
                    revision_id,
                    logical_key: record.logical_key.clone(),
                    revision_hash: record.revision_hash.clone(),
                    record_kind: record.record_kind.as_str().into(),
                    envelope_sha256: envelope_sha256.clone(),
                    source_artifact_id,
                });
            }
        }
        Some(receipts)
    };
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(((report.revisions, report.revoked), extra, committed_records))
}

pub(in crate::artifact_import) fn begin_bundle_transaction(
    connection: &Connection,
) -> Result<rusqlite::Transaction<'_>, String> {
    rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())
}
