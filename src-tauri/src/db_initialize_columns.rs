// Launch migrations that only add or verify columns: the agent-runtime fields, the
// platform schema check and the fuse-entry status repair. Included from db_initialize.rs.

/// Agent-runtime columns on a development database, the light per-table additions older releases are missing, and retry children whose parent scan was deleted.
fn migrate_legacy_agent_and_retry_columns(connection: &mut Connection) -> Result<(), String> {
    // The agent runtime tables are new; this keeps a development database that
    // already created them before the state column existed usable.
    let _ = ensure_column(
        &*connection,
        "agent_runs",
        "terminal_state",
        "TEXT NOT NULL DEFAULT ''",
    );
    let _ = connection.execute("ALTER TABLE worker_nodes ADD COLUMN last_sync_at TEXT", []);
    let _ = connection.execute(
        "ALTER TABLE asset_ownership_profiles ADD COLUMN jurisdictions_json TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE asset_ownership_profiles ADD COLUMN excluded_jurisdictions_json TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE asset_ownership_profiles ADD COLUMN shared_domains_json TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE exposure_runs ADD COLUMN stage TEXT NOT NULL DEFAULT 'queued'",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE exposure_runs ADD COLUMN current_source TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE exposure_runs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "UPDATE exposure_runs SET status='interrupted',stage='interrupted',current_source='',error=CASE WHEN error='' THEN '应用上次退出时采集尚未结束；已发现结果已保留，可重新开始采集' ELSE error END,completed_at=datetime('now','localtime') WHERE status IN ('queued','running')",
        [],
    );
    // 轻量迁移：旧版数据库缺少该列时添加，已存在时忽略 duplicate column。
    let _ = connection.execute(
        "ALTER TABLE assets ADD COLUMN probe_hash TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE assets ADD COLUMN canonical_key TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute("ALTER TABLE sentinel_targets ADD COLUMN scan_id TEXT", []);
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
        "ALTER TABLE sentinel_targets ADD COLUMN last_attempt_number INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN previous_scan_id TEXT NOT NULL DEFAULT ''",
        [],
    );
    // Retries created by older releases copied their own targets/checkpoints
    // but still pointed at the first scan for future execution. If that parent
    // was deleted, the dangling comparison link made an otherwise complete
    // child impossible to retry. New retries run in place; detach legacy links
    // whose parent no longer exists.
    let _ = connection.execute(
        "UPDATE sentinel_scans SET previous_scan_id='' WHERE trim(previous_scan_id)<>'' AND NOT EXISTS (SELECT 1 FROM sentinel_scans parent WHERE parent.id=sentinel_scans.previous_scan_id)",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN llm_requests INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN input_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN output_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN cached_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN total_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN scan_type TEXT NOT NULL DEFAULT 'web'",
        [],
    );
    // Strix 1.5 reports Oviraptor's staged evidence folder as a local target.
    // It is valid input provenance, but never a Web asset/company. Repair
    // historical rows at startup so the false group disappears before the
    // first result-sync poll; source/code targets are intentionally untouched.
    let _ = connection.execute(
        "UPDATE sentinel_findings AS finding SET target_url=COALESCE((SELECT CASE WHEN COUNT(*)=1 THEN MIN(target.url) ELSE '*' END FROM sentinel_targets AS target WHERE target.scan_id=finding.scan_id AND (lower(trim(target.url)) LIKE 'http://%' OR lower(trim(target.url)) LIKE 'https://%')),'*'),updated_at=datetime('now','localtime') WHERE finding.target_url<>'*' AND lower(trim(finding.target_url)) NOT LIKE 'http://%' AND lower(trim(finding.target_url)) NOT LIKE 'https://%' AND (finding.target_url LIKE '%/strix-jobs/%' OR finding.target_url LIKE '%strix-evidence-input%') AND EXISTS (SELECT 1 FROM sentinel_scans AS scan WHERE scan.id=finding.scan_id AND scan.scan_type='web')",
        [],
    );
    let _ = connection.execute(
        "DELETE FROM sentinel_targets WHERE lower(trim(url)) NOT LIKE 'http://%' AND lower(trim(url)) NOT LIKE 'https://%' AND EXISTS (SELECT 1 FROM sentinel_scans AS scan WHERE scan.id=sentinel_targets.scan_id AND scan.scan_type='web')",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN task_name TEXT NOT NULL DEFAULT ''",
        [],
    );
    Ok(())
}

/// Fields that first shipped on macOS. A locked or interrupted Windows upgrade must not keep an older schema, so the columns are verified instead of trusted.
fn verify_locked_platform_schema(connection: &mut Connection) -> Result<(), String> {
    // These two fields were introduced on macOS first. Verify the migration so a
    // locked or interrupted Windows upgrade cannot silently keep an older schema.
    ensure_column(
        &*connection,
        "sentinel_scans",
        "source_path",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &*connection,
        "sentinel_scans",
        "skill_names",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &*connection,
        "sentinel_scans",
        "attempt_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        &*connection,
        "browser_auth_sessions",
        "capture_previous_status",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &*connection,
        "browser_auth_sessions",
        "owner_scan_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &*connection,
        "browser_auth_sessions",
        "draft_scope_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_browser_auth_sessions_task_scope ON browser_auth_sessions(owner_scan_id,draft_scope_id,project_id,updated_at DESC)",
            [],
        )
        .map_err(|error| format!("创建任务会话作用域索引失败：{error}"))?;
    let _ = connection.execute(
        "UPDATE sentinel_scans SET attempt_count=1 WHERE attempt_count=0 AND status<>'draft'",
        [],
    );
    for sql in [
        "ALTER TABLE security_rule_packs ADD COLUMN progress INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN progress_stage TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE security_rule_packs ADD COLUMN progress_message TEXT NOT NULL DEFAULT ''",
    ] {
        let _ = connection.execute(sql, []);
    }
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sentinel_processes (
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                process_id INTEGER NOT NULL DEFAULT 0,
                engine TEXT NOT NULL DEFAULT '',
                work_dir TEXT NOT NULL DEFAULT '',
                started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                PRIMARY KEY(scan_id,process_id)
            );
            CREATE TABLE IF NOT EXISTS sentinel_scan_attempts (
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                attempt_number INTEGER NOT NULL,
                execution_mode TEXT NOT NULL DEFAULT 'initial',
                status TEXT NOT NULL DEFAULT 'scanning',
                stage TEXT NOT NULL DEFAULT 'initializing',
                checkpoint TEXT NOT NULL DEFAULT '',
                stop_reason TEXT NOT NULL DEFAULT '',
                work_dir TEXT NOT NULL DEFAULT '',
                llm_requests_start INTEGER NOT NULL DEFAULT 0,
                input_tokens_start INTEGER NOT NULL DEFAULT 0,
                output_tokens_start INTEGER NOT NULL DEFAULT 0,
                cached_tokens_start INTEGER NOT NULL DEFAULT 0,
                total_tokens_start INTEGER NOT NULL DEFAULT 0,
                llm_requests_delta INTEGER NOT NULL DEFAULT 0,
                input_tokens_delta INTEGER NOT NULL DEFAULT 0,
                output_tokens_delta INTEGER NOT NULL DEFAULT 0,
                cached_tokens_delta INTEGER NOT NULL DEFAULT 0,
                total_tokens_delta INTEGER NOT NULL DEFAULT 0,
                started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                finished_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                PRIMARY KEY(scan_id,attempt_number)
            );
            CREATE INDEX IF NOT EXISTS idx_sentinel_attempt_scan ON sentinel_scan_attempts(scan_id,attempt_number DESC);
            CREATE TABLE IF NOT EXISTS strix_skills (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                description TEXT NOT NULL DEFAULT '',
                instructions TEXT NOT NULL,
                builtin INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE TABLE IF NOT EXISTS security_rule_packs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                key TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                engine TEXT NOT NULL,
                repository TEXT NOT NULL,
                reference TEXT NOT NULL DEFAULT 'main',
                local_path TEXT NOT NULL DEFAULT '',
                previous_version TEXT NOT NULL DEFAULT '',
                version TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1,
                builtin INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'not_installed',
                last_sync_at TEXT NOT NULL DEFAULT '',
                error TEXT NOT NULL DEFAULT '',
                added_count INTEGER NOT NULL DEFAULT 0,
                modified_count INTEGER NOT NULL DEFAULT 0,
                deleted_count INTEGER NOT NULL DEFAULT 0,
                change_summary TEXT NOT NULL DEFAULT '[]',
                progress INTEGER NOT NULL DEFAULT 0,
                progress_stage TEXT NOT NULL DEFAULT '',
                progress_message TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_security_rule_packs_enabled ON security_rule_packs(enabled,engine);
            CREATE TABLE IF NOT EXISTS strix_knowledge_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scan_id TEXT NOT NULL UNIQUE,
                project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                patterns_json TEXT NOT NULL DEFAULT '{}',
                skill_instructions TEXT NOT NULL DEFAULT '',
                source_hash TEXT NOT NULL DEFAULT '',
                skill_id INTEGER REFERENCES strix_skills(id) ON DELETE SET NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_strix_knowledge_project ON strix_knowledge_entries(project_id,updated_at);
            CREATE TABLE IF NOT EXISTS strix_learning_candidates (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
                scan_type TEXT NOT NULL DEFAULT 'web',
                title TEXT NOT NULL,
                summary TEXT NOT NULL DEFAULT '',
                candidate_json TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending',
                target_skill_id INTEGER REFERENCES strix_skills(id) ON DELETE SET NULL,
                source_hash TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                reviewed_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(scan_id,source_hash)
            );
            CREATE INDEX IF NOT EXISTS idx_strix_learning_candidates_status ON strix_learning_candidates(status,updated_at);
            CREATE INDEX IF NOT EXISTS idx_strix_learning_candidates_project ON strix_learning_candidates(project_id,updated_at);
            CREATE TABLE IF NOT EXISTS sentinel_scan_contexts (
                scan_id TEXT PRIMARY KEY REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                environment TEXT NOT NULL DEFAULT '',
                auth_profile_name TEXT NOT NULL DEFAULT '',
                auth_type TEXT NOT NULL DEFAULT 'none',
                authenticated INTEGER NOT NULL DEFAULT 0,
                ci_provider TEXT NOT NULL DEFAULT '',
                repository_url TEXT NOT NULL DEFAULT '',
                branch TEXT NOT NULL DEFAULT '',
                commit_sha TEXT NOT NULL DEFAULT '',
                build_id TEXT NOT NULL DEFAULT '',
                policy_json TEXT NOT NULL DEFAULT '{}',
                gate_status TEXT NOT NULL DEFAULT 'not_evaluated',
                gate_reason TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE TABLE IF NOT EXISTS browser_auth_sessions (
                id TEXT PRIMARY KEY,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                owner_scan_id TEXT NOT NULL DEFAULT '',
                draft_scope_id TEXT NOT NULL DEFAULT '',
                name TEXT NOT NULL DEFAULT '',
                entry_url TEXT NOT NULL,
                final_url TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'capturing',
                scope_hosts_json TEXT NOT NULL DEFAULT '[]',
                cookie_count INTEGER NOT NULL DEFAULT 0,
                header_count INTEGER NOT NULL DEFAULT 0,
                storage_count INTEGER NOT NULL DEFAULT 0,
                captured_request_count INTEGER NOT NULL DEFAULT 0,
                session_json TEXT NOT NULL DEFAULT '{}',
                last_validated_at TEXT NOT NULL DEFAULT '',
                expires_at TEXT NOT NULL DEFAULT '',
                last_error TEXT NOT NULL DEFAULT '',
                capture_previous_status TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_browser_auth_sessions_project ON browser_auth_sessions(project_id,status,updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_browser_auth_sessions_task_scope ON browser_auth_sessions(owner_scan_id,draft_scope_id,project_id,updated_at DESC);
            CREATE TABLE IF NOT EXISTS appsec_vulnerabilities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                fingerprint TEXT NOT NULL,
                title TEXT NOT NULL,
                vulnerability_type TEXT NOT NULL DEFAULT '',
                severity TEXT NOT NULL DEFAULT 'info',
                status TEXT NOT NULL DEFAULT 'open',
                confidence TEXT NOT NULL DEFAULT '',
                asset TEXT NOT NULL DEFAULT '',
                environment TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL DEFAULT '',
                http_method TEXT NOT NULL DEFAULT '',
                parameter TEXT NOT NULL DEFAULT '',
                file TEXT NOT NULL DEFAULT '',
                symbol TEXT NOT NULL DEFAULT '',
                start_line INTEGER NOT NULL DEFAULT 0,
                correlation_score INTEGER NOT NULL DEFAULT 0,
                correlation_json TEXT NOT NULL DEFAULT '{}',
                first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                owner TEXT NOT NULL DEFAULT '',
                UNIQUE(project_id,fingerprint)
            );
            CREATE TABLE IF NOT EXISTS appsec_vulnerability_sources (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                vulnerability_id INTEGER NOT NULL REFERENCES appsec_vulnerabilities(id) ON DELETE CASCADE,
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                finding_id INTEGER REFERENCES sentinel_findings(id) ON DELETE CASCADE,
                source_type TEXT NOT NULL,
                source_key TEXT NOT NULL,
                engine TEXT NOT NULL DEFAULT '',
                evidence_json TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(vulnerability_id,scan_id,source_type,source_key)
            );
            CREATE INDEX IF NOT EXISTS idx_appsec_vuln_project ON appsec_vulnerabilities(project_id,last_seen);
            CREATE INDEX IF NOT EXISTS idx_appsec_source_scan ON appsec_vulnerability_sources(scan_id,vulnerability_id);
            CREATE TABLE IF NOT EXISTS sentinel_fuse_zone (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
                company TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL,
                normalized_url TEXT NOT NULL,
                source_scan_id TEXT NOT NULL DEFAULT '',
                reason TEXT NOT NULL DEFAULT '',
                verdict TEXT NOT NULL DEFAULT 'pending',
                note TEXT NOT NULL DEFAULT '',
                evidence TEXT NOT NULL DEFAULT '',
                archived INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(project_id,normalized_url)
            );
            CREATE INDEX IF NOT EXISTS idx_sentinel_fuse_project ON sentinel_fuse_zone(project_id,archived,updated_at);
            "#,
        )
        .map_err(|error| error.to_string())?;
    ensure_column(
        &*connection,
        "sentinel_scan_attempts",
        "execution_mode",
        "TEXT NOT NULL DEFAULT 'initial'",
    )?;
    Ok(())
}

/// Phase 2 and Stage 1A columns on agent_runs: the invocation id coverage cites, the per-attempt backend plan, the frozen plan, and the orchestration fields that keep every existing row on the single-agent path.
fn migrate_agent_run_orchestration_columns(connection: &mut Connection) -> Result<(), String> {
    // Coverage entries cite a tool invocation by the id the runtime handed out, so
    // the audit row and the coverage ledger agree (§9.2).
    ensure_column(
        &*connection,
        "tool_invocations",
        "invocation_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    // Phase 2 §3.2: the backend matrix is decided once per attempt, before any
    // dependency is resolved, and is never recomputed from settings that changed
    // while the attempt was running.
    ensure_column(
        &*connection,
        "sentinel_scan_attempts",
        "backend_plan_json",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    // Phase 2 §2.2: the frozen plan belongs to an attempt, not to a URL. `agent_runs`
    // already carries the attempt dimension, so the plan lives beside it and the old
    // per-URL checkpoint row stays a compatibility projection for the UI.
    ensure_column(
        &*connection,
        "agent_runs",
        "plan_json",
        "TEXT NOT NULL DEFAULT '{}'",
    )?;
    // Stage 1A: the orchestration columns belong to `agent_runs` directly, and the
    // default keeps every existing row — and every new scan — on the single-agent
    // path. History is never rewritten to `multi`.
    for (column, definition) in [
        ("root_run_id", "TEXT NOT NULL DEFAULT ''"),
        ("assignment_id", "TEXT NOT NULL DEFAULT ''"),
        ("lane", "TEXT NOT NULL DEFAULT ''"),
        ("orchestration_policy", "TEXT NOT NULL DEFAULT 'single'"),
        ("capability_lease_json", "TEXT NOT NULL DEFAULT '[]'"),
        ("reserved_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("reserved_requests", "INTEGER NOT NULL DEFAULT 0"),
        ("heartbeat_at", "TEXT NOT NULL DEFAULT ''"),
        ("cancel_requested_at", "TEXT NOT NULL DEFAULT ''"),
    ] {
        ensure_column(&*connection, "agent_runs", column, definition)?;
    }
    let _ = connection.execute(
        "INSERT OR IGNORE INTO sentinel_scan_attempts(scan_id,attempt_number,status,stage,checkpoint,stop_reason,llm_requests_delta,input_tokens_delta,output_tokens_delta,cached_tokens_delta,total_tokens_delta,started_at,finished_at,updated_at) SELECT id,MAX(attempt_count,1),status,CASE WHEN status IN ('completed','partial') THEN 'complete' WHEN status IN ('failed','cancelled') THEN 'stopped' WHEN status IN ('paused','pausing') THEN 'paused' ELSE 'unknown' END,current_checkpoint,CASE WHEN status IN ('completed','partial','failed','cancelled','paused') THEN current_checkpoint ELSE '' END,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,created_at,CASE WHEN status IN ('completed','partial','failed','cancelled','paused') THEN updated_at ELSE '' END,updated_at FROM sentinel_scans WHERE attempt_count>0",
        [],
    );
    let old_process_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='sentinel_processes'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if old_process_schema.contains("scan_id TEXT PRIMARY KEY") {
        connection
            .execute_batch(
                r#"
                ALTER TABLE sentinel_processes RENAME TO sentinel_processes_legacy;
                CREATE TABLE sentinel_processes (
                    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                    process_id INTEGER NOT NULL DEFAULT 0,
                    engine TEXT NOT NULL DEFAULT '',
                    work_dir TEXT NOT NULL DEFAULT '',
                    started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                    PRIMARY KEY(scan_id,process_id)
                );
                INSERT OR IGNORE INTO sentinel_processes(scan_id,process_id,engine,work_dir,started_at)
                  SELECT scan_id,process_id,engine,work_dir,started_at FROM sentinel_processes_legacy;
                DROP TABLE sentinel_processes_legacy;
                "#,
            )
            .map_err(|error| format!("升级扫描进程表失败：{error}"))?;
    }
    for statement in [
        "ALTER TABLE security_rule_packs ADD COLUMN added_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN modified_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN deleted_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN change_summary TEXT NOT NULL DEFAULT '[]'",
        "ALTER TABLE security_rule_packs ADD COLUMN previous_version TEXT NOT NULL DEFAULT ''",
    ] {
        let _ = connection.execute(statement, []);
    }
    connection.execute_batch(
        r#"
        INSERT OR IGNORE INTO security_rule_packs(key,name,engine,repository,reference,enabled,builtin)
        VALUES
          ('semgrep-rules','Semgrep Rules','semgrep','https://github.com/semgrep/semgrep-rules.git','develop',1,1),
          ('codeql-queries','CodeQL queries','codeql','https://github.com/github/codeql.git','main',1,1),
          ('owasp-benchmark','OWASP Benchmark','benchmark','https://github.com/OWASP/Benchmark.git','master',1,1);
        "#,
    ).map_err(|error| error.to_string())?;
    connection.execute(
        "INSERT OR IGNORE INTO strix_skills(name,description,instructions,builtin,enabled) VALUES('业务前端深度分析','按看功能、触发请求、还原参数、分析业务 JS、匹配本地知识和一次性保底发现的顺序执行；只把证据充分的高价值候选交给 Strix。',?1,1,1)",
        [DEFAULT_BUSINESS_FRONTEND_SKILL],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

/// Only confirmed protection-system blocks become persistent fuse entries; budget, context and no-progress stops remain retryable checkpoints.
fn migrate_fuse_entry_status(connection: &mut Connection) -> Result<(), String> {
    // Only confirmed protection-system blocks become persistent fuse entries.
    // Budget, context and no-progress stops remain retryable checkpoints.
    connection.execute(
        "INSERT OR IGNORE INTO sentinel_fuse_zone(project_id,asset_id,company,url,normalized_url,source_scan_id,reason) SELECT project_id,asset_id,company,url,lower(rtrim(trim(url),'/')),COALESCE(scan_id,''),routing_reason FROM sentinel_targets WHERE status='limited' AND trim(url)<>'' AND (lower(routing_reason) LIKE '%waf%' OR lower(routing_reason) LIKE '%captcha%' OR lower(routing_reason) LIKE '%cloudflare%' OR lower(routing_reason) LIKE '%rate limit%' OR routing_reason LIKE '%验证码%' OR routing_reason LIKE '%人机验证%' OR routing_reason LIKE '%持续限流%')",
        [],
    ).map_err(|error| error.to_string())?;
    if migration_version(&*connection, "soft_fuse_cleanup_version") < 1 {
        connection.execute(
            "UPDATE sentinel_fuse_zone SET archived=1,note='旧版将预算、上下文或无进展软暂停误记为熔断；现已恢复为可继续任务',updated_at=datetime('now','localtime') WHERE archived=0 AND verdict='pending' AND trim(evidence)='' AND NOT (lower(reason) LIKE '%waf%' OR lower(reason) LIKE '%captcha%' OR lower(reason) LIKE '%cloudflare%' OR lower(reason) LIKE '%rate limit%' OR reason LIKE '%验证码%' OR reason LIKE '%人机验证%' OR reason LIKE '%持续限流%')",
            [],
        ).map_err(|error| error.to_string())?;
        finish_migration(&*connection, "soft_fuse_cleanup_version", 1)?;
    }
    if migration_version(&*connection, "builtin_src_assurance_version") < 1 {
        connection.execute(
            "UPDATE config_profiles SET settings_json=json_remove(settings_json,'$.strixOastEndpoint','$.strixRawHttpEnabled','$.strixRaceEnabled','$.strixMaxRaceConcurrency','$.strixControlledWriteEnabled','$.strixAttackChainEnabled'),updated_at=datetime('now','localtime') WHERE json_valid(settings_json)",
            [],
        ).map_err(|error| error.to_string())?;
        finish_migration(&*connection, "builtin_src_assurance_version", 1)?;
    }
    if migration_version(&*connection, "strix_false_completion_repair_version") < 1 {
        connection.execute_batch(
            r#"
            UPDATE sentinel_targets
            SET status='partial',
                routing_reason=CASE
                  WHEN routing_reason LIKE '%历史修复：未取得目标工具证据%' THEN routing_reason
                  ELSE routing_reason || '；历史修复：未取得目标工具证据，不计入自动验证完成'
                END,
                updated_at=datetime('now','localtime')
            WHERE status='completed'
              AND routing_reason LIKE '%自动验证已按边界收口（本轮未形成新的工具证据）%'
              AND (
                routing_reason LIKE '%没有取得目标请求/响应%'
                OR routing_reason LIKE '%没有形成可用工具结果%'
                OR routing_reason LIKE '%没有形成任何工具证据%'
                OR routing_reason LIKE '%只读取了本地证据%'
              );

            UPDATE sentinel_scans
            SET status='partial',
                current_checkpoint='调查已收口：自动验证 '
                  || (SELECT COUNT(*) FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id AND t.status='completed')
                  || '，保留待验证 '
                  || (SELECT COUNT(*) FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id AND t.status='partial')
                  || '，仅侦察收口 '
                  || (SELECT COUNT(*) FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id AND t.status='recon_only')
                  || '；旧版曾将未取得目标请求/响应的回合误记为完成，现已校正',
                updated_at=datetime('now','localtime')
            WHERE scan_type='web'
              AND status='completed'
              AND EXISTS(
                SELECT 1 FROM sentinel_targets t
                WHERE t.scan_id=sentinel_scans.id
                  AND t.status='partial'
                  AND t.routing_reason LIKE '%历史修复：未取得目标工具证据%'
              );
            "#,
        ).map_err(|error| format!("修复 Strix 假完成历史状态失败：{error}"))?;
        finish_migration(&*connection, "strix_false_completion_repair_version", 1)?;
    }
    Ok(())
}
