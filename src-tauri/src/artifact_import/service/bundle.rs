use super::*;

fn failed_outcome(
    bundle: &Bundle,
    manifest: Option<&Manifest>,
    status: BundleStatus,
    records: usize,
    diagnostics: Vec<Diagnostic>,
) -> BundleOutcome {
    BundleOutcome {
        source_path: bundle.source_dir.display().to_string(),
        bundle_id: manifest
            .map(|row| row.bundle_id.clone())
            .unwrap_or_default(),
        status,
        records,
        #[cfg(test)]
        revisions: 0,
        #[cfg(test)]
        revoked: 0,
        diagnostics: sorted(diagnostics),
        committed_records: None,
    }
}

pub(super) fn import_bundle(
    context: &ImportContext<'_>,
    root: &Path,
    bundle: &Bundle,
) -> BundleOutcome {
    if !bundle.discovery_errors.is_empty() {
        return failed_outcome(
            bundle,
            None,
            BundleStatus::Failed,
            0,
            bundle.discovery_errors.clone(),
        );
    }
    let (manifest, payloads) = match build(root, &bundle.source_dir, &bundle.files, context.limits)
    {
        Ok(built) => built,
        Err(failures) => {
            return failed_outcome(bundle, None, BundleStatus::Failed, 0, failures);
        }
    };
    let signature = format!(
        "{}\u{1}{}\u{1}adapter={}",
        manifest.canonical,
        bundle.source_dir.display(),
        crate::artifact_import::canonical::ADAPTER_VERSION
    );
    let stored = store::find_bundle(context.connection, &manifest.bundle_id)
        .ok()
        .flatten();
    if let Some(row) = stored.filter(|row| row.signature == signature) {
        return BundleOutcome {
            source_path: bundle.source_dir.display().to_string(),
            bundle_id: manifest.bundle_id.clone(),
            status: BundleStatus::Unchanged,
            records: 0,
            #[cfg(test)]
            revisions: 0,
            #[cfg(test)]
            revoked: 0,
            diagnostics: vec![Diagnostic::info(
                "bundle_unchanged",
                bundle.source_dir.display().to_string(),
                format!("内容与签名一致（{}），本轮不重导", row.bundle_id),
            )],
            committed_records: None,
        };
    }
    let parse_context = ParseContext {
        bundle_id: &manifest.bundle_id,
        manifest: &manifest,
        payloads: &payloads,
        limits: context.limits,
    };
    let (records, mut diagnostics) = adapters::parse_bundle(&parse_context);
    let mut groups = match adapters::source_report::parse_directory(&parse_context) {
        Ok(groups) => groups,
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "source_report_invalid",
                bundle.source_dir.display().to_string(),
                error,
            ));
            return failed_outcome(
                bundle,
                Some(&manifest),
                BundleStatus::Failed,
                0,
                diagnostics,
            );
        }
    };
    // Which scan/attempt this batch belongs to decides what it may revoke; the search
    // root it happened to be found under never does.
    let scope = scope::resolve(root, bundle, &payloads);
    let input_count = records
        .len()
        .saturating_add(groups.iter().map(|g| g.records.len()).sum::<usize>());
    // §COR-007: going over a bound is a controlled refusal to write anything.
    if input_count > context.limits.records {
        diagnostics.push(Diagnostic::error(
            "record_count_limit",
            bundle.source_dir.display().to_string(),
            format!(
                "合并前 {} 条记录，超过上限 {}，本轮不写入",
                input_count, context.limits.records
            ),
        ));
        return failed_outcome(
            bundle,
            Some(&manifest),
            BundleStatus::Failed,
            input_count,
            diagnostics,
        );
    }
    if !records.is_empty() || groups.is_empty() {
        groups.push(ScopedRecords {
            scope,
            records: reconcile::merge_by_logical_key(records),
        });
    }
    // Reconcile each task/attempt once, newest attempt first. Applying each file
    // separately would revoke the preceding file's records in the same scope.
    let mut scoped =
        std::collections::BTreeMap::<(String, std::cmp::Reverse<i64>), Vec<CanonicalRecord>>::new();
    for group in groups {
        scoped
            .entry((
                group.scope.scan_id,
                std::cmp::Reverse(group.scope.attempt_number),
            ))
            .or_default()
            .extend(group.records.into_values());
    }
    let groups = scoped
        .into_iter()
        .map(|((scan_id, attempt), records)| ScopedRecords {
            scope: scope::Scope {
                scan_id,
                attempt_number: attempt.0,
            },
            records: reconcile::merge_by_logical_key(records),
        })
        .collect::<Vec<_>>();
    let record_count = groups.iter().map(|g| g.records.len()).sum();
    let cas_files = match write_objects(context, &manifest, &payloads) {
        Ok(rows) => rows,
        Err(failures) => {
            diagnostics.extend(failures);
            return failed_outcome(
                bundle,
                Some(&manifest),
                BundleStatus::Failed,
                record_count,
                diagnostics,
            );
        }
    };
    let outcome = within_busy_bound("提交 bundle", || {
        commit_bundle(
            context,
            root,
            bundle,
            &manifest,
            &signature,
            &cas_files,
            &groups,
            &diagnostics,
        )
    });
    match outcome {
        Ok(((revisions, revoked), extra, committed_records)) => {
            diagnostics.extend(extra.iter().cloned());
            BundleOutcome {
                source_path: bundle.source_dir.display().to_string(),
                bundle_id: manifest.bundle_id.clone(),
                status: status_of(&[], &diagnostics, revisions, revoked),
                records: record_count,
                #[cfg(test)]
                revisions,
                #[cfg(test)]
                revoked,
                diagnostics: sorted(diagnostics),
                committed_records,
            }
        }
        Err(error) => {
            diagnostics.push(Diagnostic::error(
                "bundle_failed",
                bundle.source_dir.display().to_string(),
                format!("事务已回滚，本 bundle 没有留下任何投影：{error}"),
            ));
            failed_outcome(
                bundle,
                Some(&manifest),
                BundleStatus::Failed,
                record_count,
                diagnostics,
            )
        }
    }
}
