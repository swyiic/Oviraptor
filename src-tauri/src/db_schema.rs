// The whole SQLite schema: every table, index and view the app creates on open.
// Included from db.rs so the schema text stays one contiguous block (§12).

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS config_profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    is_default INTEGER NOT NULL DEFAULT 0,
    settings_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS worker_nodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    endpoint TEXT NOT NULL UNIQUE,
    access_token TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_seen_at TEXT,
    last_sync_at TEXT,
    last_error TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS content_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    keyword TEXT NOT NULL,
    normalized_keyword TEXT NOT NULL UNIQUE,
    category TEXT NOT NULL DEFAULT 'custom_rule',
    source_asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_content_rules_enabled ON content_rules(enabled,normalized_keyword);

CREATE TABLE IF NOT EXISTS hackerone_programs (
    id TEXT PRIMARY KEY,
    handle TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    icon_url TEXT NOT NULL DEFAULT '',
    policy TEXT NOT NULL DEFAULT '',
    policy_hash TEXT NOT NULL DEFAULT '',
    submission_state TEXT NOT NULL DEFAULT '',
    program_state TEXT NOT NULL DEFAULT '',
    offers_bounties INTEGER NOT NULL DEFAULT 0,
    open_scope INTEGER NOT NULL DEFAULT 0,
    fast_payments INTEGER NOT NULL DEFAULT 0,
    safe_harbor INTEGER NOT NULL DEFAULT 0,
    collaboration INTEGER NOT NULL DEFAULT 0,
    started_accepting_at TEXT,
    last_synced_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    custom_industry TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS hackerone_scopes (
    id TEXT PRIMARY KEY,
    program_handle TEXT NOT NULL,
    asset_type TEXT NOT NULL DEFAULT '',
    asset_identifier TEXT NOT NULL DEFAULT '',
    eligible_for_submission INTEGER NOT NULL DEFAULT 0,
    eligible_for_bounty INTEGER NOT NULL DEFAULT 0,
    max_severity TEXT NOT NULL DEFAULT '',
    instruction TEXT NOT NULL DEFAULT '',
    reference TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS hackerone_exclusions (
    id TEXT PRIMARY KEY,
    program_handle TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    details TEXT NOT NULL DEFAULT '',
    updated_at TEXT,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS hackerone_notes (
    program_handle TEXT PRIMARY KEY,
    bookmarked INTEGER NOT NULL DEFAULT 0,
    tags TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    last_tested_at TEXT
);

CREATE TABLE IF NOT EXISTS hackerone_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    program_handle TEXT NOT NULL,
    event_type TEXT NOT NULL,
    summary TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE INDEX IF NOT EXISTS idx_h1_program_handle ON hackerone_programs(handle);
CREATE INDEX IF NOT EXISTS idx_h1_scope_program ON hackerone_scopes(program_handle,active);
CREATE INDEX IF NOT EXISTS idx_h1_event_program ON hackerone_events(program_handle,created_at);

CREATE TABLE IF NOT EXISTS sentinel_targets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    company TEXT NOT NULL DEFAULT '',
    url TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    value_score INTEGER NOT NULL DEFAULT 0,
    scan_mode TEXT NOT NULL DEFAULT '',
    routing_reason TEXT NOT NULL DEFAULT '',
    last_attempt_number INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,scan_id,url)
);
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
CREATE TABLE IF NOT EXISTS sentinel_scans (
    id TEXT PRIMARY KEY,
    project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    project_name TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'queued',
    current_checkpoint TEXT NOT NULL DEFAULT '',
    task_path TEXT NOT NULL DEFAULT '',
    previous_scan_id TEXT NOT NULL DEFAULT '',
    llm_requests INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    scan_type TEXT NOT NULL DEFAULT 'web',
    task_name TEXT NOT NULL DEFAULT '',
    source_path TEXT NOT NULL DEFAULT '',
    skill_names TEXT NOT NULL DEFAULT '',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
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
    backend_plan_json TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(scan_id,attempt_number)
);
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
CREATE TABLE IF NOT EXISTS sentinel_deleted_scans (
    scan_id TEXT PRIMARY KEY,
    deleted_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS sentinel_processes (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    process_id INTEGER NOT NULL DEFAULT 0,
    engine TEXT NOT NULL DEFAULT '',
    work_dir TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,process_id)
);
CREATE TABLE IF NOT EXISTS sentinel_checkpoints (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    stage TEXT NOT NULL,
    raw_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,url,stage)
);
CREATE TABLE IF NOT EXISTS sentinel_findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL DEFAULT '',
    stage TEXT NOT NULL,
    kind TEXT NOT NULL,
    record_key TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    severity TEXT NOT NULL DEFAULT '',
    record_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,stage,kind,record_key)
);
CREATE INDEX IF NOT EXISTS idx_sentinel_findings_scan ON sentinel_findings(scan_id,stage,kind);
CREATE TABLE IF NOT EXISTS sentinel_opportunities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL DEFAULT '',
    opportunity_key TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    score INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'queued',
    confidence TEXT NOT NULL DEFAULT '',
    why_json TEXT NOT NULL DEFAULT '[]',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    recommended_action_json TEXT NOT NULL DEFAULT '{}',
    source TEXT NOT NULL DEFAULT '',
    record_json TEXT NOT NULL DEFAULT '{}',
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,opportunity_key)
);
CREATE INDEX IF NOT EXISTS idx_sentinel_opportunities_inbox ON sentinel_opportunities(project_id,status,score DESC,last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_opportunities_scan ON sentinel_opportunities(scan_id,target_url,score DESC);
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
CREATE TABLE IF NOT EXISTS sentinel_validations (
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
CREATE INDEX IF NOT EXISTS idx_sentinel_scan_updated ON sentinel_scans(updated_at);
CREATE INDEX IF NOT EXISTS idx_sentinel_scan_project_updated ON sentinel_scans(project_id,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_attempt_scan ON sentinel_scan_attempts(scan_id,attempt_number DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_targets_project_updated ON sentinel_targets(project_id,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_targets_scan ON sentinel_targets(scan_id);
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

-- Investigation graph: deterministic browser/AST evidence is kept as first-class
-- data instead of being flattened into generic findings.  The graph is rebuilt
-- idempotently for each scan target, while baselines and learned layers retain
-- cross-scan history.
CREATE TABLE IF NOT EXISTS investigation_nodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    node_key TEXT NOT NULL,
    node_type TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    confidence TEXT NOT NULL DEFAULT '',
    value_score INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'observed',
    payload_json TEXT NOT NULL DEFAULT '{}',
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,node_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_nodes_scan ON investigation_nodes(scan_id,target_url,node_type);
CREATE INDEX IF NOT EXISTS idx_investigation_nodes_project ON investigation_nodes(project_id,node_type,last_seen DESC);

CREATE TABLE IF NOT EXISTS investigation_edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    source_key TEXT NOT NULL,
    relation TEXT NOT NULL,
    target_key TEXT NOT NULL,
    confidence TEXT NOT NULL DEFAULT '',
    evidence_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,source_key,relation,target_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_edges_scan ON investigation_edges(scan_id,target_url,source_key);

CREATE TABLE IF NOT EXISTS investigation_actions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    action_key TEXT NOT NULL,
    state_key TEXT NOT NULL DEFAULT '',
    action_type TEXT NOT NULL DEFAULT 'interaction',
    label TEXT NOT NULL DEFAULT '',
    outcome TEXT NOT NULL DEFAULT '',
    value_score INTEGER NOT NULL DEFAULT 0,
    protocol_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,action_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_actions_scan ON investigation_actions(scan_id,target_url,value_score DESC);

CREATE TABLE IF NOT EXISTS investigation_api_models (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    method TEXT NOT NULL DEFAULT 'UNKNOWN',
    url TEXT NOT NULL DEFAULT '',
    normalized_path TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL DEFAULT '',
    confidence TEXT NOT NULL DEFAULT '',
    auth_scope TEXT NOT NULL DEFAULT 'unknown',
    parameters_json TEXT NOT NULL DEFAULT '[]',
    request_schema_json TEXT NOT NULL DEFAULT '{}',
    response_schema_json TEXT NOT NULL DEFAULT '{}',
    state_keys_json TEXT NOT NULL DEFAULT '[]',
    action_keys_json TEXT NOT NULL DEFAULT '[]',
    identity_keys_json TEXT NOT NULL DEFAULT '[]',
    observed_count INTEGER NOT NULL DEFAULT 1,
    baseline_status TEXT NOT NULL DEFAULT 'new',
    payload_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,api_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_api_scan ON investigation_api_models(scan_id,target_url,baseline_status);
CREATE INDEX IF NOT EXISTS idx_investigation_api_project ON investigation_api_models(project_id,normalized_path,method);

CREATE TABLE IF NOT EXISTS investigation_hypotheses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    hypothesis_key TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'candidate',
    score INTEGER NOT NULL DEFAULT 0,
    confidence TEXT NOT NULL DEFAULT '',
    contract_json TEXT NOT NULL DEFAULT '{}',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    decision_json TEXT NOT NULL DEFAULT '{}',
    source_opportunity_key TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,hypothesis_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_hypotheses_queue ON investigation_hypotheses(project_id,status,score DESC,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_investigation_hypotheses_scan ON investigation_hypotheses(scan_id,target_url,score DESC);

-- A mutation-capable verification is disabled by default. Approval is scoped
-- to one hypothesis, endpoint/method contract, a small attempt budget and an
-- expiry time so entering the validation queue never implies broad consent.
CREATE TABLE IF NOT EXISTS investigation_mutation_approvals (
    hypothesis_id INTEGER PRIMARY KEY REFERENCES investigation_hypotheses(id) ON DELETE CASCADE,
    approved INTEGER NOT NULL DEFAULT 0,
    scope_json TEXT NOT NULL DEFAULT '{}',
    max_attempts INTEGER NOT NULL DEFAULT 1,
    note TEXT NOT NULL DEFAULT '',
    expires_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_investigation_mutation_expiry ON investigation_mutation_approvals(approved,expires_at);

CREATE TABLE IF NOT EXISTS investigation_identity_diffs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    left_identity_key TEXT NOT NULL,
    right_identity_key TEXT NOT NULL,
    difference_type TEXT NOT NULL DEFAULT '',
    risk_score INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'observed',
    matrix_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,api_key,left_identity_key,right_identity_key,difference_type)
);
CREATE INDEX IF NOT EXISTS idx_investigation_identity_scan ON investigation_identity_diffs(scan_id,target_url,risk_score DESC);

CREATE TABLE IF NOT EXISTS investigation_metrics (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    node_count INTEGER NOT NULL DEFAULT 0,
    edge_count INTEGER NOT NULL DEFAULT 0,
    state_count INTEGER NOT NULL DEFAULT 0,
    action_count INTEGER NOT NULL DEFAULT 0,
    api_count INTEGER NOT NULL DEFAULT 0,
    parameter_count INTEGER NOT NULL DEFAULT 0,
    hypothesis_count INTEGER NOT NULL DEFAULT 0,
    added_count INTEGER NOT NULL DEFAULT 0,
    changed_count INTEGER NOT NULL DEFAULT 0,
    removed_count INTEGER NOT NULL DEFAULT 0,
    duplicate_count INTEGER NOT NULL DEFAULT 0,
    information_gain INTEGER NOT NULL DEFAULT 0,
    token_worthy INTEGER NOT NULL DEFAULT 0,
    stop_reason TEXT NOT NULL DEFAULT '',
    decision_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,target_url)
);
CREATE INDEX IF NOT EXISTS idx_investigation_metrics_project ON investigation_metrics(project_id,information_gain DESC,updated_at DESC);

CREATE TABLE IF NOT EXISTS investigation_baselines (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    identity_key TEXT NOT NULL DEFAULT 'anonymous',
    source_scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    signature TEXT NOT NULL,
    api_signatures_json TEXT NOT NULL DEFAULT '[]',
    parameter_signatures_json TEXT NOT NULL DEFAULT '[]',
    metrics_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,target_url,identity_key,source_scan_id)
);
CREATE INDEX IF NOT EXISTS idx_investigation_baseline_lookup ON investigation_baselines(project_id,target_url,identity_key,created_at DESC);

CREATE TABLE IF NOT EXISTS knowledge_facts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    fact_key TEXT NOT NULL,
    fact_type TEXT NOT NULL,
    subject TEXT NOT NULL DEFAULT '',
    predicate TEXT NOT NULL DEFAULT '',
    object_json TEXT NOT NULL DEFAULT '{}',
    confidence TEXT NOT NULL DEFAULT '',
    source_scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL DEFAULT '',
    evidence_hash TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,fact_key,source_scan_id,target_url)
);
CREATE INDEX IF NOT EXISTS idx_knowledge_facts_subject ON knowledge_facts(project_id,fact_type,subject,last_seen DESC);

CREATE TABLE IF NOT EXISTS knowledge_strategies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    strategy_key TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    conditions_json TEXT NOT NULL DEFAULT '{}',
    playbook_json TEXT NOT NULL DEFAULT '{}',
    support_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    promoted INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,strategy_key)
);
CREATE INDEX IF NOT EXISTS idx_knowledge_strategies_project ON knowledge_strategies(project_id,promoted,support_count DESC);

CREATE TABLE IF NOT EXISTS knowledge_outcomes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    hypothesis_key TEXT NOT NULL DEFAULT '',
    strategy_key TEXT NOT NULL DEFAULT '',
    outcome TEXT NOT NULL DEFAULT '',
    stop_reason TEXT NOT NULL DEFAULT '',
    evidence_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,hypothesis_key,strategy_key)
);
CREATE INDEX IF NOT EXISTS idx_knowledge_outcomes_strategy ON knowledge_outcomes(project_id,strategy_key,outcome);

CREATE TABLE IF NOT EXISTS targets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    target_type TEXT NOT NULL,
    value TEXT NOT NULL,
    normalized_value TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id, target_type, normalized_value)
);

CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    profile_id INTEGER REFERENCES config_profiles(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    pipeline TEXT NOT NULL DEFAULT 'collect',
    status TEXT NOT NULL DEFAULT 'queued',
    stage TEXT NOT NULL DEFAULT 'queued',
    progress REAL NOT NULL DEFAULT 0,
    processed INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0,
    config_snapshot TEXT NOT NULL DEFAULT '{}',
    output_dir TEXT NOT NULL DEFAULT '',
    error TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    started_at TEXT,
    finished_at TEXT
);

CREATE TABLE IF NOT EXISTS assets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_key TEXT NOT NULL UNIQUE,
    company TEXT NOT NULL DEFAULT '',
    host TEXT NOT NULL DEFAULT '',
    link TEXT NOT NULL DEFAULT '',
    ip TEXT NOT NULL DEFAULT '',
    port TEXT NOT NULL DEFAULT '',
    protocol TEXT NOT NULL DEFAULT '',
    domain TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    status_code TEXT NOT NULL DEFAULT '',
    probe_outcome TEXT NOT NULL DEFAULT '',
    probe_entry_state TEXT NOT NULL DEFAULT '',
    review_tier TEXT NOT NULL DEFAULT '',
    content_category TEXT NOT NULL DEFAULT '',
    score TEXT NOT NULL DEFAULT '',
    state_hash TEXT NOT NULL DEFAULT '',
    probe_hash TEXT NOT NULL DEFAULT '',
    canonical_key TEXT NOT NULL DEFAULT '',
    extra_json TEXT NOT NULL DEFAULT '{}',
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_alive TEXT
);

CREATE TABLE IF NOT EXISTS project_assets (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    decision TEXT NOT NULL DEFAULT 'pending',
    note TEXT NOT NULL DEFAULT '',
    is_deleted INTEGER NOT NULL DEFAULT 0,
    deleted_at TEXT,
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_run_id INTEGER REFERENCES runs(id) ON DELETE SET NULL,
    PRIMARY KEY(project_id, asset_id)
);

CREATE TABLE IF NOT EXISTS asset_ownership_profiles (
    project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    legal_name TEXT NOT NULL DEFAULT '',
    jurisdiction TEXT NOT NULL DEFAULT '',
    jurisdictions_json TEXT NOT NULL DEFAULT '[]',
    excluded_jurisdictions_json TEXT NOT NULL DEFAULT '[]',
    aliases_json TEXT NOT NULL DEFAULT '[]',
    approved_domains_json TEXT NOT NULL DEFAULT '[]',
    shared_domains_json TEXT NOT NULL DEFAULT '[]',
    excluded_names_json TEXT NOT NULL DEFAULT '[]',
    excluded_domains_json TEXT NOT NULL DEFAULT '[]',
    notes TEXT NOT NULL DEFAULT '',
    policy_version INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS asset_ownership_decisions (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'unreviewed',
    confidence INTEGER NOT NULL DEFAULT 0,
    authorization_status TEXT NOT NULL DEFAULT 'unknown',
    exposure_eligible INTEGER NOT NULL DEFAULT 0,
    source TEXT NOT NULL DEFAULT 'auto',
    reason TEXT NOT NULL DEFAULT '',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    negative_evidence_json TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(project_id,asset_id)
);
CREATE INDEX IF NOT EXISTS idx_asset_ownership_queue ON asset_ownership_decisions(project_id,status,exposure_eligible,confidence DESC);

CREATE TABLE IF NOT EXISTS asset_ownership_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    rule_type TEXT NOT NULL,
    pattern TEXT NOT NULL,
    action TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'manual',
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,rule_type,pattern,action)
);
CREATE INDEX IF NOT EXISTS idx_asset_ownership_rules_project ON asset_ownership_rules(project_id,enabled,rule_type);

CREATE TABLE IF NOT EXISTS exposure_findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    category TEXT NOT NULL,
    title TEXT NOT NULL,
    source_type TEXT NOT NULL DEFAULT 'manual',
    source_url TEXT NOT NULL DEFAULT '',
    evidence_excerpt TEXT NOT NULL DEFAULT '',
    severity TEXT NOT NULL DEFAULT 'medium',
    confidence INTEGER NOT NULL DEFAULT 50,
    status TEXT NOT NULL DEFAULT 'new',
    fingerprint TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    first_seen_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,fingerprint)
);
CREATE INDEX IF NOT EXISTS idx_exposure_findings_queue ON exposure_findings(project_id,status,severity,confidence DESC);
CREATE INDEX IF NOT EXISTS idx_exposure_findings_asset ON exposure_findings(project_id,asset_id);

CREATE TABLE IF NOT EXISTS exposure_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'queued',
    eligible_assets INTEGER NOT NULL DEFAULT 0,
    scanned_assets INTEGER NOT NULL DEFAULT 0,
    fetched_resources INTEGER NOT NULL DEFAULT 0,
    findings INTEGER NOT NULL DEFAULT 0,
    stage TEXT NOT NULL DEFAULT 'queued',
    current_source TEXT NOT NULL DEFAULT '',
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    error TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    started_at TEXT NOT NULL DEFAULT '',
    completed_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_exposure_runs_project ON exposure_runs(project_id,id DESC);

CREATE TABLE IF NOT EXISTS exposure_source_results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES exposure_runs(id) ON DELETE CASCADE,
    source_key TEXT NOT NULL,
    status TEXT NOT NULL,
    item_count INTEGER NOT NULL DEFAULT 0,
    error TEXT NOT NULL DEFAULT '',
    completed_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(run_id,source_key)
);

CREATE TABLE IF NOT EXISTS asset_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    run_id INTEGER REFERENCES runs(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER REFERENCES runs(id) ON DELETE CASCADE,
    level TEXT NOT NULL DEFAULT 'info',
    stage TEXT NOT NULL DEFAULT '',
    message TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS saved_views (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    columns_json TEXT NOT NULL DEFAULT '[]',
    filters_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE INDEX IF NOT EXISTS idx_targets_project ON targets(project_id);
CREATE INDEX IF NOT EXISTS idx_runs_project_created ON runs(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_assets_ip ON assets(ip);
CREATE INDEX IF NOT EXISTS idx_assets_domain ON assets(domain);
CREATE INDEX IF NOT EXISTS idx_assets_host ON assets(host);
CREATE INDEX IF NOT EXISTS idx_assets_probe ON assets(probe_outcome);
CREATE INDEX IF NOT EXISTS idx_assets_tier ON assets(review_tier);
CREATE INDEX IF NOT EXISTS idx_assets_review_order ON assets(review_tier, score, last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_project_assets_project_deleted ON project_assets(project_id, is_deleted);
CREATE INDEX IF NOT EXISTS idx_project_assets_project_decision ON project_assets(project_id, is_deleted, decision, last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_project_assets_asset ON project_assets(asset_id);
CREATE INDEX IF NOT EXISTS idx_events_project_created ON asset_events(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_logs_run_created ON logs(run_id, created_at DESC);

CREATE TABLE IF NOT EXISTS agent_runs (
    id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL DEFAULT 1,
    target_url TEXT NOT NULL DEFAULT '',
    backend TEXT NOT NULL DEFAULT 'strix',
    role TEXT NOT NULL DEFAULT 'coordinator',
    parent_run_id TEXT REFERENCES agent_runs(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'prepared',
    plan_hash TEXT NOT NULL DEFAULT '',
    evidence_hash TEXT NOT NULL DEFAULT '',
    soft_token_budget INTEGER NOT NULL DEFAULT 0,
    hard_token_budget INTEGER NOT NULL DEFAULT 0,
    used_tokens INTEGER NOT NULL DEFAULT 0,
    used_cached_tokens INTEGER NOT NULL DEFAULT 0,
    soft_request_budget INTEGER NOT NULL DEFAULT 0,
    hard_request_budget INTEGER NOT NULL DEFAULT 0,
    used_requests INTEGER NOT NULL DEFAULT 0,
    lease_expires_at TEXT NOT NULL DEFAULT '',
    terminal_reason TEXT NOT NULL DEFAULT '',
    terminal_code TEXT NOT NULL DEFAULT '',
    terminal_state TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    started_at TEXT NOT NULL DEFAULT '',
    finished_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS agent_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    artifact_refs_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(run_id, sequence)
);

CREATE TABLE IF NOT EXISTS agent_messages (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    from_agent TEXT NOT NULL DEFAULT '',
    to_agent TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL,
    correlation_id TEXT NOT NULL DEFAULT '',
    dedup_key TEXT NOT NULL DEFAULT '',
    payload_json TEXT NOT NULL DEFAULT '{}',
    artifact_refs_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    delivered_at TEXT NOT NULL DEFAULT '',
    acknowledged_at TEXT NOT NULL DEFAULT '',
    UNIQUE(run_id, dedup_key)
);

CREATE TABLE IF NOT EXISTS tool_invocations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    tool_version INTEGER NOT NULL DEFAULT 1,
    contract_key TEXT NOT NULL DEFAULT '',
    identity_handle TEXT NOT NULL DEFAULT '',
    input_hash TEXT NOT NULL DEFAULT '',
    input_summary_json TEXT NOT NULL DEFAULT '{}',
    policy_decision TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'running',
    progress_signature TEXT NOT NULL DEFAULT '',
    request_artifact_id TEXT NOT NULL DEFAULT '',
    response_artifact_id TEXT NOT NULL DEFAULT '',
    error_class TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    finished_at TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS agent_snapshots (
    run_id TEXT PRIMARY KEY REFERENCES agent_runs(id) ON DELETE CASCADE,
    last_sequence INTEGER NOT NULL DEFAULT 0,
    schema_version INTEGER NOT NULL DEFAULT 1,
    snapshot_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

-- Stage 1A: the multi-agent skeleton. Nothing schedules or runs from these tables
-- yet; they exist so a later stage stores an assignment, the shared evidence graph
-- and reviewer decisions without inventing storage on the fly.
CREATE TABLE IF NOT EXISTS agent_assignments (
    id TEXT PRIMARY KEY,
    coordinator_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    child_run_id TEXT NOT NULL DEFAULT '',
    role TEXT NOT NULL DEFAULT 'coordinator',
    lane TEXT NOT NULL DEFAULT 'read_only_analysis',
    target_key TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'prepared',
    dedup_key TEXT NOT NULL,
    trigger_code TEXT NOT NULL DEFAULT '',
    task_slice_json TEXT NOT NULL DEFAULT '{}',
    evidence_revision INTEGER NOT NULL DEFAULT 0,
    contract_keys_json TEXT NOT NULL DEFAULT '[]',
    identity_handles_json TEXT NOT NULL DEFAULT '[]',
    reserved_tokens INTEGER NOT NULL DEFAULT 0,
    reserved_requests INTEGER NOT NULL DEFAULT 0,
    capability_lease_json TEXT NOT NULL DEFAULT '[]',
    deadline_at TEXT NOT NULL DEFAULT '',
    failure_class TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    leased_at TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT '',
    finished_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(coordinator_run_id, dedup_key)
);

CREATE TABLE IF NOT EXISTS agent_evidence_nodes (
    id TEXT PRIMARY KEY,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 1,
    kind TEXT NOT NULL DEFAULT 'target',
    provenance TEXT NOT NULL DEFAULT 'observed',
    natural_key_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    artifact_refs_json TEXT NOT NULL DEFAULT '[]',
    created_by_run_id TEXT NOT NULL DEFAULT '',
    supersedes_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(root_run_id, revision, natural_key_hash)
);

CREATE TABLE IF NOT EXISTS agent_evidence_edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 1,
    from_node_id TEXT NOT NULL REFERENCES agent_evidence_nodes(id) ON DELETE CASCADE,
    to_node_id TEXT NOT NULL REFERENCES agent_evidence_nodes(id) ON DELETE CASCADE,
    kind TEXT NOT NULL DEFAULT 'derived_from',
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_by_run_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(root_run_id, revision, from_node_id, to_node_id, kind)
);

CREATE TABLE IF NOT EXISTS agent_review_decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    candidate_id TEXT NOT NULL,
    candidate_revision INTEGER NOT NULL DEFAULT 1,
    reviewer_run_id TEXT NOT NULL,
    verdict TEXT NOT NULL DEFAULT 'insufficient_evidence',
    reason_codes_json TEXT NOT NULL DEFAULT '[]',
    evidence_refs_json TEXT NOT NULL DEFAULT '[]',
    counter_evidence_refs_json TEXT NOT NULL DEFAULT '[]',
    missing_evidence_json TEXT NOT NULL DEFAULT '[]',
    confidence REAL NOT NULL DEFAULT 0,
    severity TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(candidate_id, candidate_revision, reviewer_run_id)
);

CREATE INDEX IF NOT EXISTS idx_agent_runs_scan ON agent_runs(scan_id,attempt_number,target_url);
CREATE INDEX IF NOT EXISTS idx_agent_runs_parent ON agent_runs(parent_run_id,status);
CREATE INDEX IF NOT EXISTS idx_agent_runs_lease ON agent_runs(status,lease_expires_at);
CREATE INDEX IF NOT EXISTS idx_agent_messages_run_delivered ON agent_messages(run_id,delivered_at,created_at);
CREATE INDEX IF NOT EXISTS idx_tool_invocations_run_status ON tool_invocations(run_id,status,started_at DESC);
CREATE INDEX IF NOT EXISTS idx_tool_invocations_contract ON tool_invocations(run_id,contract_key);
CREATE INDEX IF NOT EXISTS idx_agent_assignments_coordinator ON agent_assignments(coordinator_run_id,state,lane);
CREATE INDEX IF NOT EXISTS idx_agent_assignments_child ON agent_assignments(child_run_id);
CREATE INDEX IF NOT EXISTS idx_agent_evidence_nodes_revision ON agent_evidence_nodes(root_run_id,revision,kind);
CREATE INDEX IF NOT EXISTS idx_agent_evidence_edges_revision ON agent_evidence_edges(root_run_id,revision,from_node_id);
CREATE INDEX IF NOT EXISTS idx_agent_review_candidate ON agent_review_decisions(candidate_id,candidate_revision);
"#;
