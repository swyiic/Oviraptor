//! Task-owned, bounded reference views of public bundled fingerprints only.
//! This module never loads user rule files, probes a URL, or changes a task.
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

const MAX_OFFSET: usize = 5 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 4096;
const MAX_RESULT_BYTES: usize = 24 * 1024;
fn default_search_limit() -> usize {
    10
}
fn default_read_limit() -> usize {
    MAX_PAGE_BYTES
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Search {
    task_id: String,
    query: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_search_limit")]
    limit: usize,
    #[serde(default)]
    catalog_revision: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Read {
    task_id: String,
    fingerprint_id: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_read_limit")]
    limit: usize,
    #[serde(default)]
    catalog_revision: String,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_text(value: &str, bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= bytes && !value.chars().any(char::is_control)
}
fn revision_shape(args: &Value, revision: &str, offset: usize) -> Result<(), String> {
    if args.get("catalogRevision").is_some() {
        if revision.len() != 64 || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("scan_arguments_invalid".into());
        }
    } else if offset > 0 {
        return Err("scan_fingerprint_revision_required".into());
    }
    Ok(())
}
fn search_input(args: &Value) -> Result<Search, String> {
    let input: Search =
        serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
    if !valid_text(&input.task_id, 128)
        || !valid_text(&input.query, 640)
        || input.query.chars().count() > 160
        || input.limit == 0
        || input.limit > 20
        || input.offset > 5000
    {
        return Err("scan_arguments_invalid".into());
    }
    revision_shape(args, &input.catalog_revision, input.offset)?;
    Ok(input)
}
fn read_input(args: &Value) -> Result<Read, String> {
    let input: Read = serde_json::from_value(args.clone()).map_err(|_| "scan_arguments_invalid")?;
    if !valid_text(&input.task_id, 128)
        || input.fingerprint_id.is_empty()
        || input.fingerprint_id.len() > 128
        || !input
            .fingerprint_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || input.offset > MAX_OFFSET
        || input.limit == 0
        || input.limit > MAX_PAGE_BYTES
    {
        return Err("scan_arguments_invalid".into());
    }
    revision_shape(args, &input.catalog_revision, input.offset)?;
    Ok(input)
}
pub(super) fn validate_arguments(action: &str, args: &Value) -> Result<(), String> {
    match action {
        "fingerprint_search" => {
            search_input(args)?;
        }
        "fingerprint_read" => {
            read_input(args)?;
        }
        _ => return Err("scan_action_invalid".into()),
    }
    Ok(())
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| {
            let code = ch as u32;
            let ch = if (0xff01..=0xff5e).contains(&code) {
                char::from_u32(code - 0xfee0).unwrap_or(ch)
            } else if ch == '\u{3000}' {
                ' '
            } else {
                ch
            };
            ch.to_lowercase()
        })
        .collect()
}
fn searchable(value: &Value, output: &mut String) {
    match value {
        Value::String(value) => {
            output.push_str(value);
            output.push('\n');
        }
        Value::Array(values) => {
            for value in values {
                searchable(value, output);
            }
        }
        Value::Object(values) => {
            for (name, value) in values {
                output.push_str(name);
                output.push('\n');
                searchable(value, output);
            }
        }
        _ => {}
    }
}
fn leaf_count(condition: &Value) -> usize {
    condition
        .get("children")
        .and_then(Value::as_array)
        .map(|children| children.iter().map(leaf_count).sum())
        .unwrap_or(1)
}
fn signal_summary(rule: &Value) -> Value {
    let array_len = |name| rule.get(name).and_then(Value::as_array).map_or(0, Vec::len);
    let condition = rule.get("condition");
    json!({
        "mode": condition.and_then(|v| v["type"].as_str()).unwrap_or("legacy-any"),
        "conditionLeaves": condition.map_or(0, leaf_count),
        "headerPatterns":rule.get("headerRegex").and_then(Value::as_object).map_or(0, |v|v.len()),
        "cookiePatterns":array_len("setCookieRegex"),"bodyPatterns":array_len("bodyRegex"),
        "pathSignals":array_len("pathSignals")
    })
}

struct Rule {
    id: String,
    name: String,
    text: String,
    sha256: String,
    search: String,
    summary: Value,
}
struct Catalog {
    revision: String,
    rules: Vec<Rule>,
}
impl Catalog {
    fn from_values(mut values: Vec<Value>) -> Result<Self, String> {
        values.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        let mut ids = std::collections::HashSet::new();
        let revision = digest(
            &serde_json::to_vec(&values).map_err(|_| "scan_fingerprint_catalog_unavailable")?,
        );
        let mut rules = Vec::with_capacity(values.len());
        for value in values {
            let id = value["id"]
                .as_str()
                .ok_or("scan_fingerprint_catalog_unavailable")?
                .to_string();
            let name = value["name"]
                .as_str()
                .ok_or("scan_fingerprint_catalog_unavailable")?
                .to_string();
            if !ids.insert(id.clone()) {
                return Err("scan_fingerprint_catalog_unavailable".into());
            }
            let text = serde_json::to_string(&value)
                .map_err(|_| "scan_fingerprint_catalog_unavailable")?;
            if text.len() > MAX_OFFSET {
                return Err("scan_fingerprint_rule_too_large".into());
            }
            let mut search = String::new();
            searchable(&value, &mut search);
            rules.push(Rule {
                id,
                name,
                sha256: digest(text.as_bytes()),
                text,
                search: normalize(&search),
                summary: signal_summary(&value),
            });
        }
        Ok(Self { revision, rules })
    }
    fn check_revision(&self, value: &str) -> Result<(), String> {
        if !value.is_empty() && !value.eq_ignore_ascii_case(&self.revision) {
            return Err("scan_fingerprint_catalog_changed".into());
        }
        Ok(())
    }
    fn envelope(&self, project: i64, task: &str) -> Value {
        json!({"projectId":project,"taskId":task,"scope":"bundled-defaults", "userOverridesIncluded":false,
            "catalogRevision":self.revision,"totalCatalog":self.rules.len(),"readOnly":true,
            "untrusted":true,"authority":"reference_only_no_execution","targetRequests":0,
            "source":"bundled_fingerprint_resources","independentTargetVerification":false})
    }
    fn search(&self, project: i64, input: Search) -> Result<Value, String> {
        self.check_revision(&input.catalog_revision)?;
        let query = normalize(input.query.trim());
        let matches: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| r.search.contains(&query))
            .collect();
        if input.offset > matches.len() {
            return Err("scan_fingerprint_offset_invalid".into());
        }
        let end = input.offset.saturating_add(input.limit).min(matches.len());
        let rows: Vec<Value> = matches[input.offset..end]
            .iter()
            .map(|rule| json!({"id":rule.id,"name":rule.name,"signalSummary":rule.summary}))
            .collect();
        let mut output = self.envelope(project, &input.task_id);
        output["query"] = json!(input.query);
        output["fingerprints"] = json!(rows);
        output["totalMatches"] = json!(matches.len());
        output["offset"] = json!(input.offset);
        output["returned"] = json!(end - input.offset);
        output["hasMore"] = json!(end < matches.len());
        output["nextOffset"] = if end < matches.len() {
            json!(end)
        } else {
            Value::Null
        };
        bounded(output)
    }
    fn read(&self, project: i64, input: Read) -> Result<Value, String> {
        self.check_revision(&input.catalog_revision)?;
        let rule = self
            .rules
            .iter()
            .find(|rule| rule.id == input.fingerprint_id)
            .ok_or("scan_fingerprint_unavailable")?;
        if input.offset > rule.text.len() || !rule.text.is_char_boundary(input.offset) {
            return Err("scan_fingerprint_offset_invalid".into());
        }
        let mut end = input
            .offset
            .saturating_add(input.limit)
            .min(rule.text.len());
        while end > input.offset && !rule.text.is_char_boundary(end) {
            end -= 1;
        }
        if end == input.offset && input.offset < rule.text.len() {
            return Err("scan_fingerprint_page_too_small".into());
        }
        let mut output = self.envelope(project, &input.task_id);
        output["fingerprintId"] = json!(rule.id);
        output["name"] = json!(rule.name);
        output["encoding"] = json!("utf8-json");
        output["sha256"] = json!(rule.sha256);
        output["totalBytes"] = json!(rule.text.len());
        output["offset"] = json!(input.offset);
        output["content"] = json!(&rule.text[input.offset..end]);
        output["hasMore"] = json!(end < rule.text.len());
        output["nextOffset"] = if end < rule.text.len() {
            json!(end)
        } else {
            Value::Null
        };
        bounded(output)
    }
}
fn bounded(output: Value) -> Result<Value, String> {
    if serde_json::to_vec(&output)
        .map_err(|_| "scan_fingerprint_catalog_unavailable")?
        .len()
        > MAX_RESULT_BYTES
    {
        return Err("scan_fingerprint_result_too_large".into());
    }
    Ok(output)
}
fn catalog() -> Result<&'static Catalog, String> {
    static CATALOG: OnceLock<Result<Catalog, String>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let value = crate::scanning::fingerprint_pack::bundled_fingerprints_json()?;
            Catalog::from_values(
                value
                    .as_array()
                    .ok_or("scan_fingerprint_catalog_unavailable")?
                    .clone(),
            )
        })
        .as_ref()
        .map_err(Clone::clone)
}
pub(super) fn dispatch(
    c: &rusqlite::Connection,
    project: i64,
    action: &str,
    args: &Value,
) -> Result<Value, String> {
    validate_arguments(action, args)?;
    let task = args["taskId"].as_str().ok_or("scan_arguments_invalid")?;
    super::present(c, project)?;
    super::owned_scope(c, project, task)?;
    // The provider is exclusively embedded resources: no app_data_dir or user file API.
    match action {
        "fingerprint_search" => catalog()?.search(project, search_input(args)?),
        "fingerprint_read" => catalog()?.read(project, read_input(args)?),
        _ => Err("scan_action_invalid".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connection() -> rusqlite::Connection {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,status TEXT);CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY,project_id INTEGER);CREATE TABLE sentinel_deleted_scans(scan_id TEXT);INSERT INTO projects VALUES(1,'active'),(2,'active');INSERT INTO sentinel_scans VALUES('mine',1),('foreign',2),('deleted',1);INSERT INTO sentinel_deleted_scans VALUES('deleted');").unwrap();
        c
    }
    fn sample() -> Catalog {
        Catalog::from_values(vec![
        json!({"id":"frame-a","name":"框架Ａ","condition":{"type":"all","children":[{"type":"body","pattern":"共同标记"},{"type":"header","name":"server","pattern":"Vendor-A"}]}}),
        json!({"id":"frame-b","name":"框架Ｂ","bodyRegex":["共同标记"]})
    ]).unwrap()
    }
    #[test]
    fn public_provider_is_task_scoped_and_never_writes() {
        let c = connection();
        let before: i64 = c
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        let data = dispatch(
            &c,
            1,
            "fingerprint_search",
            &json!({"taskId":"mine","query":"nginx"}),
        )
        .unwrap();
        assert_eq!(data["scope"], "bundled-defaults");
        assert_eq!(data["userOverridesIncluded"], false);
        assert_eq!(data["targetRequests"], 0);
        assert_eq!(data["authority"], "reference_only_no_execution");
        assert_eq!(data["totalCatalog"], catalog().unwrap().rules.len());
        assert!(data["fingerprints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == "nginx"));
        let after: i64 = c
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after);
        assert!(!data.to_string().contains("userPath"));
        assert!(!data.to_string().contains("exposures"));
        for (project, task) in [(1, "foreign"), (2, "mine"), (1, "deleted"), (3, "mine")] {
            assert!(dispatch(
                &c,
                project,
                "fingerprint_search",
                &json!({"taskId":task,"query":"nginx"})
            )
            .is_err());
        }
    }
    #[test]
    fn strict_inputs_reject_wrong_types_excess_and_unknown_fields() {
        for value in [
            json!({}),
            json!({"taskId":"mine","query":""}),
            json!({"taskId":"mine","query":null}),
            json!({"taskId":"mine","query":"x","offset":-1}),
            json!({"taskId":"mine","query":"x","limit":1.1}),
            json!({"taskId":"mine","query":"x","limit":0}),
            json!({"taskId":"mine","query":"x","limit":21}),
            json!({"taskId":"mine","query":"x","offset":null}),
            json!({"taskId":"mine","query":"x","projectId":2}),
            json!({"taskId":"mine","query":"x","catalogRevision":null}),
            json!({"taskId":"mine","query":"x","catalogRevision":""}),
            json!({"taskId":"mine","query":"x".repeat(161)}),
            json!({"taskId":"mine\n","query":"x"}),
        ] {
            assert!(
                validate_arguments("fingerprint_search", &value).is_err(),
                "{value}"
            );
        }
        for value in [
            json!({"taskId":"mine","fingerprintId":"../private"}),
            json!({"taskId":"mine","fingerprintId":"nginx","limit":4097}),
            json!({"taskId":"mine","fingerprintId":"nginx","limit":null}),
            json!({"taskId":"mine","fingerprintId":"nginx","offset":MAX_OFFSET+1}),
            json!({"taskId":"mine","fingerprintId":"nginx","file":"private"}),
        ] {
            assert!(
                validate_arguments("fingerprint_read", &value).is_err(),
                "{value}"
            );
        }
    }
    #[test]
    fn search_pages_by_stable_catalog_and_keeps_real_counts() {
        let cat = sample();
        let first = cat
            .search(
                1,
                search_input(&json!({"taskId":"mine","query":"共同","limit":1})).unwrap(),
            )
            .unwrap();
        assert_eq!(first["totalCatalog"], 2);
        assert_eq!(first["totalMatches"], 2);
        assert_eq!(first["nextOffset"], 1);
        assert_eq!(first["returned"], 1);
        let second=cat.search(1,search_input(&json!({"taskId":"mine","query":"共同","limit":1,"offset":1,"catalogRevision":cat.revision})).unwrap()).unwrap();
        assert_eq!(second["fingerprints"][0]["id"], "frame-b");
        assert_eq!(second["hasMore"], false);
        assert!(search_input(&json!({"taskId":"mine","query":"共同","offset":1})).is_err());
        assert!(cat
            .search(
                1,
                search_input(
                    &json!({"taskId":"mine","query":"共同","catalogRevision":"0".repeat(64)})
                )
                .unwrap()
            )
            .is_err());
        let normalized = cat
            .search(
                1,
                search_input(&json!({"taskId":"mine","query":"框架a"})).unwrap(),
            )
            .unwrap();
        assert_eq!(normalized["totalMatches"], 1);
    }
    #[test]
    fn read_reassembles_utf8_json_without_flattening_condition_and_verifies_hash() {
        let cat = sample();
        let rule = &cat.rules[0];
        let mut bytes = String::new();
        let mut offset = 0;
        loop {
            let page=cat.read(1,read_input(&json!({"taskId":"mine","fingerprintId":rule.id,"offset":offset,"limit":19,"catalogRevision":cat.revision})).unwrap()).unwrap();
            assert_eq!(page["sha256"], rule.sha256);
            assert_eq!(page["totalBytes"], rule.text.len());
            let text = page["content"].as_str().unwrap();
            assert!(text.len() <= 19);
            bytes.push_str(text);
            match page["nextOffset"].as_u64() {
                Some(next) => {
                    assert!(next as usize > offset);
                    offset = next as usize;
                }
                None => break,
            }
        }
        assert_eq!(digest(bytes.as_bytes()), rule.sha256);
        assert_eq!(bytes, rule.text);
        let parsed: Value = serde_json::from_str(&bytes).unwrap();
        assert_eq!(parsed["condition"]["type"], "all");
        assert_eq!(parsed["condition"]["children"].as_array().unwrap().len(), 2);
        let inside = rule.text.find('框').unwrap() + 1;
        assert!(cat.read(1,read_input(&json!({"taskId":"mine","fingerprintId":rule.id,"offset":inside,"catalogRevision":cat.revision})).unwrap()).is_err());
        assert!(cat
            .read(
                1,
                read_input(&json!({"taskId":"mine","fingerprintId":"absent"})).unwrap()
            )
            .is_err());
    }
}
