// Launch migrations that rewrite historical rows: built-in prompts and recon routes,
// target/opportunity reclassification, investigation source keys. Included from
// db_initialize.rs.

/// Refresh the shipped built-in prompt without touching user-authored skills, and re-add the adaptive routing fields after the legacy table rebuild.
fn migrate_builtin_prompts_and_recon_routes(connection: &mut Connection) -> Result<(), String> {
    // Refresh the shipped built-in prompt in older databases without touching
    // user-authored skills.
    let _ = connection.execute(
        "UPDATE strix_skills SET description='按看功能、触发请求、还原参数、分析业务 JS、匹配本地知识和一次性保底发现的顺序执行；只把证据充分的高价值候选交给 Strix。',instructions=?1,updated_at=datetime('now','localtime') WHERE name='业务前端深度分析' AND builtin=1 AND instructions<>?1",
        [DEFAULT_BUSINESS_FRONTEND_SKILL],
    );
    let old_target_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='sentinel_targets'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if old_target_schema.contains("UNIQUE(project_id,url)") {
        let _ = connection.execute_batch(r#"
            ALTER TABLE sentinel_targets RENAME TO sentinel_targets_legacy;
            CREATE TABLE sentinel_targets (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                scan_id TEXT REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
                company TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(project_id,scan_id,url)
            );
            INSERT INTO sentinel_targets(id,project_id,scan_id,asset_id,company,url,status,created_at,updated_at)
              SELECT id,project_id,scan_id,NULL,company,url,status,created_at,updated_at FROM sentinel_targets_legacy;
            DROP TABLE sentinel_targets_legacy;
        "#);
    }
    // 旧版表重建后再次补齐自适应路由字段；新版数据库上的重复列错误可忽略。
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN value_score INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN scan_mode TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN routing_reason TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN last_attempt_number INTEGER NOT NULL DEFAULT 0",
        [],
    );
    backfill_sentinel_target_attempts(&*connection)?;
    repair_latest_attempt_summaries(&*connection)?;
    if migration_version(&*connection, "canonical_key_backfill_version") < 1 {
        connection
            .execute(
                "UPDATE assets SET canonical_key=lower(rtrim(CASE WHEN trim(link)<>'' THEN trim(link) WHEN trim(host)<>'' THEN trim(host) WHEN trim(ip)<>'' OR trim(port)<>'' THEN trim(protocol)||'|'||trim(ip)||'|'||trim(port) ELSE asset_key END,'/')) WHERE canonical_key=''",
                [],
            )
            .map_err(|error| error.to_string())?;
        finish_migration(&*connection, "canonical_key_backfill_version", 1)?;
    }
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_assets_canonical ON assets(canonical_key)",
            [],
        )
        .map_err(|error| error.to_string())?;
    connection.execute(
        "CREATE INDEX IF NOT EXISTS idx_sentinel_targets_asset ON sentinel_targets(project_id,asset_id)",
        [],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

/// Historical target-to-asset repair plus the opportunity reclassifications: inferred paths, static clues and transport identifiers are no longer verifiable work.
fn migrate_targets_and_opportunities(connection: &mut Connection) -> Result<(), String> {
    // Historical target-to-asset repair is a migration, not launch-time
    // maintenance. The old correlated query scanned the full assets table for
    // every unmatched URL and repeated forever for legitimate URL-only scans.
    // canonical_key makes the one-time lookup indexed; unresolved targets stay
    // URL-only and are not retried on every application start.
    if migration_version(&*connection, "sentinel_asset_backfill_version") < 1 {
        connection.execute(
            "UPDATE sentinel_targets AS st SET asset_id=(SELECT a.id FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=st.project_id AND a.canonical_key=lower(rtrim(trim(st.url),'/')) ORDER BY a.id LIMIT 1) WHERE st.asset_id IS NULL",
            [],
        ).map_err(|error| error.to_string())?;
        finish_migration(&*connection, "sentinel_asset_backfill_version", 1)?;
    }
    let old_validation_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='sentinel_validations'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if old_validation_schema.contains("UNIQUE(scan_id,url)") {
        connection.execute_batch(r#"
            ALTER TABLE sentinel_validations RENAME TO sentinel_validations_legacy;
            CREATE TABLE sentinel_validations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                url TEXT NOT NULL,
                finding_key TEXT NOT NULL DEFAULT 'url-summary',
                finding_kind TEXT NOT NULL DEFAULT '',
                verdict TEXT NOT NULL DEFAULT 'pending',
                severity TEXT NOT NULL DEFAULT '',
                note TEXT NOT NULL DEFAULT '',
                evidence TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(scan_id,url,finding_key)
            );
            INSERT INTO sentinel_validations(id,scan_id,url,finding_key,finding_kind,verdict,severity,note,evidence,created_at,updated_at)
              SELECT id,scan_id,url,'url-summary','',verdict,severity,note,evidence,created_at,updated_at FROM sentinel_validations_legacy;
            DROP TABLE sentinel_validations_legacy;
            CREATE INDEX IF NOT EXISTS idx_sentinel_validation_scan ON sentinel_validations(scan_id,updated_at);
CREATE TABLE IF NOT EXISTS investigation_validations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    opportunity_id INTEGER REFERENCES sentinel_opportunities(id) ON DELETE SET NULL,
    hypothesis_id INTEGER REFERENCES investigation_hypotheses(id) ON DELETE SET NULL,
    api_key TEXT NOT NULL DEFAULT '',
    identity_id TEXT NOT NULL DEFAULT '',
    method TEXT NOT NULL DEFAULT 'GET',
    request_url TEXT NOT NULL DEFAULT '',
    request_headers_json TEXT NOT NULL DEFAULT '{}',
    request_body TEXT NOT NULL DEFAULT '',
    response_status INTEGER NOT NULL DEFAULT 0,
    response_status_text TEXT NOT NULL DEFAULT '',
    response_headers_json TEXT NOT NULL DEFAULT '{}',
    response_body TEXT NOT NULL DEFAULT '',
    decoded_body TEXT NOT NULL DEFAULT '',
    verdict TEXT NOT NULL DEFAULT 'needs_more_evidence',
    severity TEXT NOT NULL DEFAULT 'info',
    confidence TEXT NOT NULL DEFAULT 'low',
    ai_assessment TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    next_action TEXT NOT NULL DEFAULT '',
    evidence_refs_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_investigation_validations_scan ON investigation_validations(scan_id,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_investigation_validations_opportunity ON investigation_validations(opportunity_id,updated_at DESC);
        "#).map_err(|error| error.to_string())?;
    }
    // Older opportunity scoring treated inferred paths, frontend routes and
    // fingerprint knowledge as directly verifiable. Reclassify them once as
    // evidence-enrichment work. Only a concrete request contract or fresh
    // runtime/probe response may remain in the Strix verification queue.
    if migration_version(&*connection, "opportunity_readiness_gate_version") < 2 {
        connection.execute_batch(r#"
            UPDATE sentinel_opportunities
            SET status='queued',
                record_json=CASE WHEN json_valid(record_json) THEN json_set(
                    record_json,
                    '$.candidateOnly', CASE WHEN source='evidence-reconstruction' THEN json('true') ELSE COALESCE(json_extract(record_json,'$.candidateOnly'),json('false')) END,
                    '$.readiness.stage', CASE
                        WHEN category='frontend_feature' THEN 'needs_runtime'
                        WHEN category='product_match' THEN 'template_match'
                        ELSE 'needs_contract' END,
                    '$.readiness.reason', CASE
                        WHEN category='frontend_feature' THEN 'frontend_route_must_be_rendered_before_security_validation'
                        WHEN category='product_match' THEN 'fingerprint_selects_a_poc_but_is_not_vulnerability_evidence'
                        ELSE 'inferred_candidate_missing_verified_method_or_response' END
                ) ELSE record_json END,
                last_seen=datetime('now','localtime')
            WHERE status IN ('ready','in_progress') AND (
                category IN ('frontend_feature','product_match','fallback_discovery')
                OR source IN ('evidence-reconstruction','string-heuristic','regex-fallback','route-structure-fallback','fingerprint')
                OR upper(COALESCE(json_extract(record_json,'$.method'),'')) IN ('','UNKNOWN')
            );

            UPDATE investigation_hypotheses AS hypothesis
            SET status='candidate',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','historical_inferred_candidate_reclassified'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('ready','in_progress') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.opportunity_key=hypothesis.source_opportunity_key
                  AND opportunity.status='queued'
            );
        "#).map_err(|error| format!("升级机会验证门禁失败：{error}"))?;
        finish_migration(&*connection, "opportunity_readiness_gate_version", 2)?;
    }
    // UNKNOWN static paths are retained in the raw frontend evidence, but they
    // are not user-facing opportunities and must never compete for model
    // budget. Earlier builds left them as queued cards, which made string
    // search output look like captured HTTP traffic.
    if migration_version(&*connection, "opportunity_readiness_gate_version") < 3 {
        connection
            .execute_batch(
                r#"
            UPDATE sentinel_opportunities
            SET status='dismissed',
                record_json=CASE WHEN json_valid(record_json) THEN json_set(
                    record_json,
                    '$.disposition','static_clue_only',
                    '$.readiness.stage','static_clue',
                    '$.readiness.reason','missing_observed_or_verified_http_method'
                ) ELSE record_json END,
                last_seen=datetime('now','localtime')
            WHERE status IN ('queued','ready')
              AND upper(COALESCE(json_extract(record_json,'$.method'),'')) IN ('','UNKNOWN')
              AND source<>'runtime-request'
              AND COALESCE(json_extract(record_json,'$.verification.verified'),0)<>1
              AND COALESCE(json_extract(record_json,'$.requestContext.status'),0)<=0;

            UPDATE investigation_hypotheses AS hypothesis
            SET status='rejected',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','historical_unknown_static_clue_removed_from_queue'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('candidate','ready','needs_more_evidence') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.opportunity_key=hypothesis.source_opportunity_key
                  AND opportunity.status='dismissed'
                  AND json_extract(opportunity.record_json,'$.disposition')='static_clue_only'
            );
        "#,
            )
            .map_err(|error| format!("升级静态接口线索门禁失败：{error}"))?;
        finish_migration(&*connection, "opportunity_readiness_gate_version", 3)?;
    }
    // A captured or AST-derived HTTP request proves endpoint existence, not a
    // vulnerability hypothesis. Retire historical ready items that lack the
    // deterministic risk signal introduced by gate v4. The raw API evidence
    // remains intact and validated/manual conclusions are never touched.
    if migration_version(&*connection, "opportunity_readiness_gate_version") < 4 {
        connection
            .execute_batch(
                r#"
            UPDATE sentinel_opportunities
            SET status='dismissed',
                record_json=CASE WHEN json_valid(record_json) THEN json_set(
                    record_json,
                    '$.disposition','api_inventory_only',
                    '$.readiness.stage','inventory_only',
                    '$.readiness.reason','formal_api_without_security_risk_signal'
                ) ELSE record_json END,
                last_seen=datetime('now','localtime')
            WHERE status='ready'
              AND COALESCE(json_extract(record_json,'$.riskEvidence.present'),0)<>1;

            UPDATE investigation_hypotheses AS hypothesis
            SET status='rejected',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','historical_formal_api_without_security_risk_signal'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('candidate','ready','needs_more_evidence') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.opportunity_key=hypothesis.source_opportunity_key
                  AND opportunity.status='dismissed'
                  AND json_extract(opportunity.record_json,'$.disposition')='api_inventory_only'
            );
        "#,
            )
            .map_err(|error| format!("升级正式接口风险门禁失败：{error}"))?;
        finish_migration(&*connection, "opportunity_readiness_gate_version", 4)?;
    }
    // Generic transport identifiers such as device_id/client_id identify the
    // browser instance, not an application-owned object. Older v4 scoring
    // treated the `_id` suffix itself as an IDOR signal and promoted ordinary
    // session recovery/callback traffic. Retire only untouched active rows;
    // manual and terminal decisions remain authoritative.
    if migration_version(&*connection, "opportunity_readiness_gate_version") < 5 {
        connection
            .execute_batch(
                r#"
            UPDATE sentinel_opportunities
            SET status='dismissed',
                record_json=CASE WHEN json_valid(record_json) THEN json_set(
                    record_json,
                    '$.disposition','transport_identifier_only',
                    '$.readiness.stage','inventory_only',
                    '$.readiness.reason','transport_identifier_is_not_an_object_authorization_boundary'
                ) ELSE record_json END,
                last_seen=datetime('now','localtime')
            WHERE status IN ('queued','ready')
              AND json_valid(record_json)
              AND COALESCE(json_extract(record_json,'$.riskEvidence.signalCount'),0)=1
              AND json_extract(record_json,'$.riskEvidence.signals[0].type')='object_boundary_parameter'
              AND NOT EXISTS (
                  SELECT 1
                  FROM json_each(json_extract(record_json,'$.riskEvidence.signals[0].fields')) AS field
                  WHERE lower(CAST(field.value AS TEXT)) NOT IN (
                      'device_id','deviceid','client_id','clientid','request_id','requestid',
                      'trace_id','traceid','session_id','sessionid','nonce','hkey','_time',
                      'timestamp','version','web_version','x_client_version'
                  )
              );

            UPDATE investigation_hypotheses AS hypothesis
            SET status='rejected',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','transport_identifier_removed_from_security_queue'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('candidate','ready','needs_more_evidence') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.opportunity_key=hypothesis.source_opportunity_key
                  AND opportunity.status='dismissed'
                  AND json_extract(opportunity.record_json,'$.disposition')='transport_identifier_only'
            );
        "#,
            )
            .map_err(|error| format!("升级传输标识误报门禁失败：{error}"))?;
        finish_migration(&*connection, "opportunity_readiness_gate_version", 5)?;
    }
    Ok(())
}

/// Investigation hypotheses use a stable category|method|path source key while the inbox stored a hashed per-observation key; both reconciliations run here.
fn reconcile_investigation_source_keys(connection: &mut Connection) -> Result<(), String> {
    // Investigation hypotheses use a stable category|method|path source key,
    // while the inbox historically stored a hashed per-observation key. Keep
    // the graph and Action Center consistent after the transport-ID cleanup.
    if migration_version(&*connection, "opportunity_readiness_gate_version") < 6 {
        connection
            .execute_batch(
                r#"
            UPDATE investigation_hypotheses AS hypothesis
            SET status='rejected',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','transport_identifier_removed_from_security_queue'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('candidate','ready','needs_more_evidence') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.status='dismissed'
                  AND json_extract(opportunity.record_json,'$.disposition')='transport_identifier_only'
                  AND lower(opportunity.category)||'|'||upper(COALESCE(json_extract(opportunity.record_json,'$.method'),''))||'|'||lower(COALESCE(json_extract(opportunity.record_json,'$.normalizedPath'),''))
                      = lower(hypothesis.source_opportunity_key)
            );

            UPDATE investigation_metrics AS metric
            SET hypothesis_count=(
                    SELECT COUNT(*) FROM investigation_hypotheses AS hypothesis
                    WHERE hypothesis.scan_id=metric.scan_id
                      AND hypothesis.target_url=metric.target_url
                      AND hypothesis.status IN ('ready','in_progress')
                ),
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.readyHypotheses',(
                        SELECT COUNT(*) FROM investigation_hypotheses AS hypothesis
                        WHERE hypothesis.scan_id=metric.scan_id
                          AND hypothesis.target_url=metric.target_url
                          AND hypothesis.status IN ('ready','in_progress')
                    ),
                    '$.eligibleForModel',CASE WHEN EXISTS (
                        SELECT 1 FROM investigation_hypotheses AS hypothesis
                        WHERE hypothesis.scan_id=metric.scan_id
                          AND hypothesis.target_url=metric.target_url
                          AND hypothesis.status IN ('ready','in_progress')
                    ) THEN json('true') ELSE json('false') END
                ),
                updated_at=datetime('now','localtime');
        "#,
            )
            .map_err(|error| format!("升级机会与调查图谱对账失败：{error}"))?;
        finish_migration(&*connection, "opportunity_readiness_gate_version", 6)?;
    }
    // v6 compared a mixed-case method segment against a lower-cased source
    // key. Repeat the reconciliation with one canonical lower-case expression.
    if migration_version(&*connection, "opportunity_readiness_gate_version") < 7 {
        connection
            .execute_batch(
                r#"
            UPDATE investigation_hypotheses AS hypothesis
            SET status='rejected',
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.eligibleForModel',json('false'),
                    '$.reason','transport_identifier_removed_from_security_queue'
                ),
                updated_at=datetime('now','localtime')
            WHERE status IN ('candidate','ready','needs_more_evidence') AND EXISTS (
                SELECT 1 FROM sentinel_opportunities AS opportunity
                WHERE opportunity.scan_id=hypothesis.scan_id
                  AND opportunity.target_url=hypothesis.target_url
                  AND opportunity.status='dismissed'
                  AND json_extract(opportunity.record_json,'$.disposition')='transport_identifier_only'
                  AND lower(opportunity.category||'|'||COALESCE(json_extract(opportunity.record_json,'$.method'),'')||'|'||COALESCE(json_extract(opportunity.record_json,'$.normalizedPath'),''))
                      = lower(hypothesis.source_opportunity_key)
            );

            UPDATE investigation_metrics AS metric
            SET hypothesis_count=(
                    SELECT COUNT(*) FROM investigation_hypotheses AS hypothesis
                    WHERE hypothesis.scan_id=metric.scan_id
                      AND hypothesis.target_url=metric.target_url
                      AND hypothesis.status IN ('ready','in_progress')
                ),
                decision_json=json_set(
                    CASE WHEN json_valid(decision_json) THEN decision_json ELSE '{}' END,
                    '$.readyHypotheses',(
                        SELECT COUNT(*) FROM investigation_hypotheses AS hypothesis
                        WHERE hypothesis.scan_id=metric.scan_id
                          AND hypothesis.target_url=metric.target_url
                          AND hypothesis.status IN ('ready','in_progress')
                    ),
                    '$.eligibleForModel',CASE WHEN EXISTS (
                        SELECT 1 FROM investigation_hypotheses AS hypothesis
                        WHERE hypothesis.scan_id=metric.scan_id
                          AND hypothesis.target_url=metric.target_url
                          AND hypothesis.status IN ('ready','in_progress')
                    ) THEN json('true') ELSE json('false') END
                ),
                updated_at=datetime('now','localtime');
        "#,
            )
            .map_err(|error| format!("修复机会与调查图谱大小写对账失败：{error}"))?;
        finish_migration(&*connection, "opportunity_readiness_gate_version", 7)?;
    }
    if migration_version(&*connection, "automatic_contract_authorization_version") < 1 {
        connection
            .execute_batch(
                r#"
                UPDATE investigation_hypotheses
                SET status=CASE
                      WHEN status IN ('awaiting_authorization','blocked_by_authorization') THEN 'ready'
                      ELSE status
                    END,
                    contract_json=CASE
                      WHEN json_valid(contract_json) THEN json_set(
                        contract_json,
                        '$.mutationPolicy',CASE COALESCE(json_extract(contract_json,'$.mutationPolicy'),'')
                          WHEN 'read_only_unless_explicitly_approved' THEN 'automatic_bounded_same_contract'
                          WHEN 'benign_marker_only_and_cleanup' THEN 'automatic_benign_marker_and_cleanup'
                          WHEN 'discovery_only_no_account_creation' THEN 'automatic_discovery_no_account_creation'
                          WHEN 'read_only_or_non_destructive' THEN 'automatic_bounded_non_destructive'
                          ELSE COALESCE(json_extract(contract_json,'$.mutationPolicy'),'automatic_bounded_non_destructive')
                        END
                      )
                      ELSE json_object('mutationPolicy','automatic_bounded_non_destructive')
                    END,
                    decision_json=CASE
                      WHEN json_valid(decision_json) THEN json_set(
                        decision_json,
                        '$.requiresHuman',json('false'),
                        '$.authorizationMode','automatic_bounded',
                        '$.verificationMode',CASE
                          WHEN status IN ('awaiting_authorization','blocked_by_authorization') THEN 'ai_auto'
                          ELSE COALESCE(json_extract(decision_json,'$.verificationMode'),'ai_auto')
                        END
                      )
                      ELSE json_object(
                        'requiresHuman',json('false'),
                        'authorizationMode','automatic_bounded',
                        'verificationMode','ai_auto'
                      )
                    END,
                    updated_at=datetime('now','localtime')
                WHERE status IN ('ready','in_progress','awaiting_authorization','blocked_by_authorization');
                "#,
            )
            .map_err(|error| format!("迁移 AI 自动验证授权策略失败：{error}"))?;
        finish_migration(&*connection, "automatic_contract_authorization_version", 1)?;
    }
    // 配置 JSON 使用向前兼容的字段级迁移，不覆盖用户已有值。
    let _ = connection.execute(
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.replaceDefaultContentRules',json('false')) WHERE json_valid(settings_json) AND json_type(settings_json,'$.replaceDefaultContentRules') IS NULL",
        [],
    );
    connection
        .execute(
            "INSERT OR IGNORE INTO app_settings(key,value) VALUES('reminder_days','7')",
            [],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT OR IGNORE INTO app_settings(key,value) VALUES('custom_icon','false')",
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}
