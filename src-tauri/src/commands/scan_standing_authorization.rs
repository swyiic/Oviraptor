// Operator "standing" authorization: a durable, pre-registered opt-in that lets
// a matching scan be auto-authorized without a live panel click (design
// §5.3 / SCAN_AGENT_AUTH_PANEL_DESIGN.md). It is written ONLY by the operator's
// Tauri command `set_scan_standing_authorization`; the agent has no tool that
// writes it, cannot widen a target pattern, and cannot sign the attestation. A
// match reuses the operator's earlier attestation to write the ordinary
// immutable per-scan record (attested_by='operator_standing') via the same
// `set_scan_capability_authorization_on` writer — so every downstream gate keeps
// reading the one immutable source of truth. Opting into any high-risk class
// still requires the operator's explicit legal-authorization attestation.
const SCAN_STANDING_AUTH_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS scan_standing_authorizations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    target_pattern TEXT NOT NULL,
    authorized_classes TEXT NOT NULL CHECK(json_valid(authorized_classes)),
    legal_attestation INTEGER NOT NULL CHECK(legal_attestation IN (0,1)),
    budget INTEGER CHECK(budget IS NULL OR budget > 0),
    budget_used INTEGER NOT NULL DEFAULT 0 CHECK(budget_used >= 0),
    expires_at TEXT,
    attested_by TEXT NOT NULL DEFAULT 'operator',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
"#;

/// A registered pattern is a bare host (`example.com`) or a wildcard family
/// (`*.example.com`). Both sides are lowercased and their trailing dot stripped.
/// A wildcard matches the apex and every subdomain of the family, because the
/// operator authorized that whole family; a bare host matches only itself. An
/// empty or malformed pattern/host never matches.
fn standing_pattern_matches(pattern: &str, host: &str) -> bool {
    let norm = |value: &str| value.trim().trim_end_matches('.').to_ascii_lowercase();
    let (pattern, host) = (norm(pattern), norm(host));
    if pattern.is_empty() || host.is_empty() {
        return false;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        if suffix.is_empty() {
            return false;
        }
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        host == pattern
    }
}

/// Validate a pattern the operator may register: a bare host or `*.host`, with
/// only characters a hostname (or that single leading wildcard label) can hold.
fn valid_standing_pattern(pattern: &str) -> bool {
    let body = pattern.strip_prefix("*.").unwrap_or(pattern);
    !body.is_empty()
        && body.len() <= 253
        && !body.starts_with('.')
        && !body.ends_with('.')
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn set_scan_standing_authorization_on(
    connection: &rusqlite::Connection,
    target_pattern: &str,
    authorized_classes: &[String],
    legal_attestation: bool,
    budget: Option<i64>,
    expires_at: Option<&str>,
) -> Result<i64, String> {
    let pattern = target_pattern.trim().trim_end_matches('.').to_ascii_lowercase();
    if !valid_standing_pattern(&pattern) {
        return Err("目标模式非法".into());
    }
    let mut classes: Vec<String> = authorized_classes.to_vec();
    classes.sort();
    classes.dedup();
    if classes.is_empty() {
        return Err("长期授权必须至少包含一个能力类".into());
    }
    if classes.iter().any(|c| !HIGH_RISK_CLASS_STRINGS.contains(&c.as_str())) {
        return Err("存在未知的高危能力类".into());
    }
    // The legal-authorization attestation is the hard precondition for any
    // high-risk standing opt-in, exactly as for a per-scan authorization.
    if !legal_attestation {
        return Err("登记长期授权前必须先确认已获得对这些目标的合法授权".into());
    }
    if budget.is_some_and(|value| value <= 0) {
        return Err("预算必须为正".into());
    }
    if let Some(expiry) = expires_at {
        let ok: bool = connection
            .query_row("SELECT datetime(?1) IS NOT NULL", [expiry], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if !ok {
            return Err("过期时间非法".into());
        }
    }
    connection
        .execute_batch(SCAN_STANDING_AUTH_SCHEMA)
        .map_err(|_| "scan_standing_authorization_storage_unavailable")?;
    let classes_json = serde_json::to_string(&classes).map_err(|e| e.to_string())?;
    connection
        .execute(
            "INSERT INTO scan_standing_authorizations(target_pattern,authorized_classes,legal_attestation,budget,expires_at) VALUES(?1,?2,1,?3,?4)",
            params![pattern, classes_json, budget, expires_at],
        )
        .map_err(|e| e.to_string())?;
    Ok(connection.last_insert_rowid())
}

/// One live standing authorization row (attested, unexpired, under budget).
struct StandingRow {
    id: i64,
    pattern: String,
    classes: Vec<String>,
}

/// Read every attested, unexpired, under-budget standing authorization. Fails
/// closed (empty) when the table is absent.
fn live_standing_authorizations(
    connection: &rusqlite::Connection,
) -> Result<Vec<StandingRow>, String> {
    let storage: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='scan_standing_authorizations')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !storage {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare(
            "SELECT id, target_pattern, authorized_classes FROM scan_standing_authorizations \
             WHERE legal_attestation=1 \
             AND (expires_at IS NULL OR datetime(expires_at) > datetime('now')) \
             AND (budget IS NULL OR budget_used < budget)",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, pattern, classes_json) = row.map_err(|e| e.to_string())?;
        let classes: Vec<String> = serde_json::from_str(&classes_json).unwrap_or_default();
        out.push(StandingRow { id, pattern, classes });
    }
    Ok(out)
}

/// Classes a single host is covered for by standing authorizations: the union of
/// every matching row's classes. Empty when nothing matches.
fn standing_classes_for_host(rows: &[StandingRow], host: &str) -> std::collections::BTreeSet<String> {
    let mut classes = std::collections::BTreeSet::new();
    for row in rows {
        if standing_pattern_matches(&row.pattern, host) {
            classes.extend(row.classes.iter().cloned());
        }
    }
    classes
}

/// Auto-apply standing authorization to a scan attempt, keyed on the scan's own
/// targets. Conservative by construction:
/// - every target host must be covered, and
/// - only the INTERSECTION of each host's covered classes is applied,
/// so an applied class is one the operator authorized for *every* target in the
/// scan. Writes the ordinary immutable record (attested_by='operator_standing')
/// and consumes one unit of budget from each row that matched a target. Does
/// nothing (and never writes an empty record) when there is no full coverage or
/// when a per-scan authorization already exists. Returns the applied classes.
pub(crate) fn apply_standing_authorizations_for_scan(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
) -> Result<Vec<String>, String> {
    // Never disturb an authorization that already exists for this attempt (the
    // operator may have used the live panel). The record is immutable anyway.
    let existing: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='scan_capability_authorizations')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if existing {
        let has: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM scan_capability_authorizations WHERE scan_id=?1 AND attempt_number=?2)",
                params![scan_id, attempt_number],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if has {
            return Ok(Vec::new());
        }
    }
    let rows = live_standing_authorizations(connection)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    // Distinct target hosts of this scan.
    let mut statement = connection
        .prepare("SELECT DISTINCT url FROM sentinel_targets WHERE scan_id=?1")
        .map_err(|e| e.to_string())?;
    let urls = statement
        .query_map([scan_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|e| e.to_string())?;
    let mut hosts: Vec<String> = Vec::new();
    for url in &urls {
        match reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)) {
            Some(host) if !host.is_empty() => {
                if !hosts.contains(&host) {
                    hosts.push(host);
                }
            }
            // A target with no parseable host cannot be covered, so the scan is
            // not fully covered and nothing is auto-applied.
            _ => return Ok(Vec::new()),
        }
    }
    if hosts.is_empty() {
        return Ok(Vec::new());
    }
    // Intersection of every host's covered classes.
    let mut applied: Option<std::collections::BTreeSet<String>> = None;
    for host in &hosts {
        let host_classes = standing_classes_for_host(&rows, host);
        if host_classes.is_empty() {
            return Ok(Vec::new());
        }
        applied = Some(match applied {
            None => host_classes,
            Some(acc) => acc.intersection(&host_classes).cloned().collect(),
        });
    }
    let applied: Vec<String> = applied.unwrap_or_default().into_iter().collect();
    if applied.is_empty() {
        return Ok(Vec::new());
    }
    set_scan_capability_authorization_on(
        connection,
        scan_id,
        attempt_number,
        true,
        &applied,
        "operator_standing",
    )?;
    // Consume one unit of budget from each row that matched any target host.
    for row in &rows {
        if hosts.iter().any(|host| standing_pattern_matches(&row.pattern, host)) {
            connection
                .execute(
                    "UPDATE scan_standing_authorizations SET budget_used=budget_used+1 WHERE id=?1",
                    [row.id],
                )
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(applied)
}

/// Operator registers a durable authorization for a target pattern. Operator-only
/// Tauri command; the agent has no tool that reaches it. Non-empty classes
/// require the legal-authorization attestation. Returns the new row id.
#[tauri::command]
pub fn set_scan_standing_authorization(
    state: State<AppState>,
    target_pattern: String,
    authorized_classes: Vec<String>,
    legal_attestation: bool,
    budget: Option<i64>,
    expires_at: Option<String>,
) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    set_scan_standing_authorization_on(
        &connection,
        &target_pattern,
        &authorized_classes,
        legal_attestation,
        budget,
        expires_at.as_deref(),
    )
}

/// Operator reads their registered standing authorizations (for the config page).
#[tauri::command]
pub fn list_scan_standing_authorizations(state: State<AppState>) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    let storage: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='scan_standing_authorizations')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !storage {
        return Ok(serde_json::json!({ "authorizations": [] }));
    }
    let mut statement = connection
        .prepare(
            "SELECT id, target_pattern, authorized_classes, budget, budget_used, expires_at, attested_by, created_at \
             FROM scan_standing_authorizations ORDER BY created_at DESC, id DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "targetPattern": r.get::<_, String>(1)?,
                "classes": serde_json::from_str::<Vec<String>>(&r.get::<_, String>(2)?).unwrap_or_default(),
                "budget": r.get::<_, Option<i64>>(3)?,
                "budgetUsed": r.get::<_, i64>(4)?,
                "expiresAt": r.get::<_, Option<String>>(5)?,
                "attestedBy": r.get::<_, String>(6)?,
                "createdAt": r.get::<_, String>(7)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    let list: Vec<JsonValue> = rows.collect::<rusqlite::Result<_>>().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "authorizations": list }))
}

/// Operator revokes a standing authorization by id. Already-written per-scan
/// records stay (they are immutable); this only stops future auto-apply.
#[tauri::command]
pub fn delete_scan_standing_authorization(state: State<AppState>, id: i64) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    connection
        .execute("DELETE FROM scan_standing_authorizations WHERE id=?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod scan_standing_authorization_tests {
    use super::{
        apply_standing_authorizations_for_scan, set_scan_standing_authorization_on,
        standing_pattern_matches,
    };
    use rusqlite::Connection;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY);
             CREATE TABLE sentinel_targets(scan_id TEXT, url TEXT);
             INSERT INTO sentinel_scans(id) VALUES('scan-a');",
        )
        .unwrap();
        c
    }

    fn target(c: &Connection, scan: &str, url: &str) {
        c.execute("INSERT INTO sentinel_targets(scan_id,url) VALUES(?1,?2)", [scan, url]).unwrap();
    }

    #[test]
    fn pattern_matching_exact_and_wildcard() {
        assert!(standing_pattern_matches("example.com", "example.com"));
        assert!(standing_pattern_matches("Example.com.", "example.com"));
        assert!(!standing_pattern_matches("example.com", "a.example.com"));
        assert!(standing_pattern_matches("*.example.com", "a.example.com"));
        assert!(standing_pattern_matches("*.example.com", "example.com"));
        assert!(standing_pattern_matches("*.example.com", "a.b.example.com"));
        assert!(!standing_pattern_matches("*.example.com", "notexample.com"));
        assert!(!standing_pattern_matches("example.com", ""));
    }

    #[test]
    fn registration_requires_attestation_and_known_classes() {
        let c = db();
        assert!(set_scan_standing_authorization_on(&c, "example.com", &["code_exec".into()], false, None, None).is_err());
        assert!(set_scan_standing_authorization_on(&c, "example.com", &["make_coffee".into()], true, None, None).is_err());
        assert!(set_scan_standing_authorization_on(&c, "bad pattern", &["code_exec".into()], true, None, None).is_err());
        assert!(set_scan_standing_authorization_on(&c, "example.com", &[], true, None, None).is_err());
        assert!(set_scan_standing_authorization_on(&c, "example.com", &["code_exec".into()], true, None, None).is_ok());
    }

    #[test]
    fn apply_authorizes_a_matching_single_target_scan() {
        let c = db();
        set_scan_standing_authorization_on(&c, "*.example.com", &["code_exec".into(), "internal_scope".into()], true, None, None).unwrap();
        target(&c, "scan-a", "https://api.example.com/login");
        let applied = apply_standing_authorizations_for_scan(&c, "scan-a", 1).unwrap();
        assert_eq!(applied, vec!["code_exec".to_string(), "internal_scope".to_string()]);
        let (att, classes): (i64, String) = c
            .query_row("SELECT legal_attestation, authorized_classes FROM scan_capability_authorizations WHERE scan_id='scan-a' AND attempt_number=1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(att, 1);
        assert!(classes.contains("internal_scope"));
        let who: String = c.query_row("SELECT attested_by FROM scan_capability_authorizations WHERE scan_id='scan-a'", [], |r| r.get(0)).unwrap();
        assert_eq!(who, "operator_standing");
        // Budget consumed on the matching row.
        let used: i64 = c.query_row("SELECT budget_used FROM scan_standing_authorizations WHERE target_pattern='*.example.com'", [], |r| r.get(0)).unwrap();
        assert_eq!(used, 1);
    }

    #[test]
    fn apply_does_nothing_when_a_target_is_not_covered() {
        let c = db();
        set_scan_standing_authorization_on(&c, "example.com", &["code_exec".into()], true, None, None).unwrap();
        target(&c, "scan-a", "https://example.com/");
        target(&c, "scan-a", "https://other.test/"); // not covered
        let applied = apply_standing_authorizations_for_scan(&c, "scan-a", 1).unwrap();
        assert!(applied.is_empty());
        // The early return never even creates the capability table — nothing was
        // written, so the scan falls back to the panel path.
        let table: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='scan_capability_authorizations')", [], |r| r.get(0)).unwrap();
        assert!(!table, "no record is written when coverage is partial — falls back to the panel");
    }

    #[test]
    fn apply_uses_the_intersection_across_targets() {
        let c = db();
        set_scan_standing_authorization_on(&c, "a.example.com", &["code_exec".into(), "egress".into()], true, None, None).unwrap();
        set_scan_standing_authorization_on(&c, "b.example.com", &["code_exec".into()], true, None, None).unwrap();
        target(&c, "scan-a", "https://a.example.com/");
        target(&c, "scan-a", "https://b.example.com/");
        let applied = apply_standing_authorizations_for_scan(&c, "scan-a", 1).unwrap();
        // egress only covers a.example.com, so it is NOT applied scan-wide.
        assert_eq!(applied, vec!["code_exec".to_string()]);
    }

    #[test]
    fn expired_authorization_does_not_apply() {
        let c = db();
        set_scan_standing_authorization_on(&c, "example.com", &["code_exec".into()], true, None, Some("2000-01-01T00:00:00Z")).unwrap();
        target(&c, "scan-a", "https://example.com/");
        assert!(apply_standing_authorizations_for_scan(&c, "scan-a", 1).unwrap().is_empty());
    }

    #[test]
    fn exhausted_budget_does_not_apply() {
        let c = db();
        set_scan_standing_authorization_on(&c, "example.com", &["code_exec".into()], true, Some(1), None).unwrap();
        c.execute("UPDATE scan_standing_authorizations SET budget_used=1", []).unwrap();
        target(&c, "scan-a", "https://example.com/");
        assert!(apply_standing_authorizations_for_scan(&c, "scan-a", 1).unwrap().is_empty());
    }

    #[test]
    fn apply_leaves_an_existing_authorization_untouched() {
        let c = db();
        // A panel authorization already exists for this attempt.
        c.execute_batch(
            "CREATE TABLE scan_capability_authorizations(scan_id TEXT, attempt_number INTEGER, legal_attestation INTEGER, authorized_classes TEXT, attested_by TEXT DEFAULT '', created_at TEXT DEFAULT '');
             INSERT INTO scan_capability_authorizations(scan_id,attempt_number,legal_attestation,authorized_classes,attested_by) VALUES('scan-a',1,1,'[\"target_write\"]','operator');",
        ).unwrap();
        set_scan_standing_authorization_on(&c, "example.com", &["code_exec".into()], true, None, None).unwrap();
        target(&c, "scan-a", "https://example.com/");
        let applied = apply_standing_authorizations_for_scan(&c, "scan-a", 1).unwrap();
        assert!(applied.is_empty(), "the immutable operator record wins; standing auth does not overwrite");
        let classes: String = c.query_row("SELECT authorized_classes FROM scan_capability_authorizations WHERE scan_id='scan-a'", [], |r| r.get(0)).unwrap();
        assert!(classes.contains("target_write") && !classes.contains("code_exec"));
    }
}
