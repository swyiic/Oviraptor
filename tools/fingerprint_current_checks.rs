// Appended inside a source-extracted module by run_fingerprint_pack_cli.py.
// This adapter never changes the application module and never sends a request.

fn checked_current_rules(text: &str) -> Result<Vec<Fingerprint>, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let array = if let Some(object) = value.as_object() {
        for key in object.keys() {
            if ![
                "schemaVersion",
                "booleanRuntime",
                "kind",
                "manifest",
                "sha256",
                "note",
                "fingerprints",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("Unknown catalog field for current runtime: {key}"));
            }
        }
        if object
            .get("schemaVersion")
            .is_some_and(|v| v != 1 && v != 2)
        {
            return Err("Current catalog schemaVersion must be 1 or 2".into());
        }
        if object
            .get("kind")
            .is_some_and(|v| v != "oviraptor-fingerprints")
        {
            return Err("Unknown fingerprint pack kind".into());
        }
        object
            .get("fingerprints")
            .and_then(serde_json::Value::as_array)
            .ok_or("Expected fingerprints array")?
    } else {
        value.as_array().ok_or("Expected catalog object or array")?
    };
    for item in array {
        let object = item.as_object().ok_or("Fingerprint must be an object")?;
        for key in object.keys() {
            if ![
                "id",
                "name",
                "headerRegex",
                "setCookieRegex",
                "bodyRegex",
                "pathSignals",
                "condition",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("Current runtime does not support fingerprint field {key}; refusing silent semantic loss"));
            }
        }
        if let Some(paths) = object.get("pathSignals") {
            for path in paths.as_array().ok_or("pathSignals must be an array")? {
                for key in path
                    .as_object()
                    .ok_or("Path signal must be an object")?
                    .keys()
                {
                    if !["path", "status", "bodyRegex"].contains(&key.as_str()) {
                        return Err(format!("Unknown current-runtime path field: {key}"));
                    }
                }
            }
        }
    }
    // Exact extracted application parser, including its native ID/duplicate checks.
    parse_user_catalog(text)
}

pub(crate) fn validate_current(text: &str) -> Result<serde_json::Value, String> {
    let rules = checked_current_rules(text)?;
    Ok(
        serde_json::json!({"valid":true,"count":rules.len(),"runtimeSemantics":"legacy-any+condition-tree-v1",
        "sourceSha256":CURRENT_SOURCE_SHA256,"sourceFile":CURRENT_SOURCE_PATH,
        "snapshotScope":"exact current model/parser/matcher functions; excludes Tauri and database recording"}),
    )
}

fn regex_witness(pattern: &str) -> Result<String, String> {
    use regex_syntax::hir::{Class, Hir, HirKind};
    fn sample(hir: &Hir) -> Result<String, String> {
        match hir.kind() {
            HirKind::Empty | HirKind::Look(_) => Ok(String::new()),
            HirKind::Literal(literal) => {
                String::from_utf8(literal.0.to_vec()).map_err(|_| "Non-UTF8 witness".into())
            }
            HirKind::Class(Class::Unicode(class)) => {
                let preferred = ['x', 'a', 'A', '0', ' ', '>', '/'];
                let character = preferred
                    .into_iter()
                    .find(|candidate| {
                        class
                            .ranges()
                            .iter()
                            .any(|range| range.start() <= *candidate && *candidate <= range.end())
                    })
                    .or_else(|| class.ranges().first().map(|range| range.start()))
                    .ok_or("Empty Unicode class")?;
                Ok(character.to_string())
            }
            HirKind::Class(Class::Bytes(class)) => {
                let byte = [b'x', b'a', b'A', b'0', b' ', b'>', b'/']
                    .into_iter()
                    .find(|candidate| {
                        class
                            .ranges()
                            .iter()
                            .any(|range| range.start() <= *candidate && *candidate <= range.end())
                    })
                    .or_else(|| class.ranges().first().map(|range| range.start()))
                    .ok_or("Empty byte class")?;
                String::from_utf8(vec![byte]).map_err(|_| "Non-UTF8 class witness".into())
            }
            HirKind::Capture(capture) => sample(&capture.sub),
            HirKind::Repetition(repetition) => {
                if repetition.min > 64 {
                    return Err("Witness repetition limit exceeded".into());
                }
                Ok(sample(&repetition.sub)?.repeat(repetition.min as usize))
            }
            HirKind::Concat(parts) => {
                let mut text = String::new();
                for part in parts {
                    text.push_str(&sample(part)?);
                    if text.len() > MAX_BODY {
                        return Err("Witness body limit exceeded".into());
                    }
                }
                Ok(text)
            }
            HirKind::Alternation(parts) => sample(parts.first().ok_or("Empty alternation")?),
        }
    }
    let hir = regex_syntax::Parser::new()
        .parse(pattern)
        .map_err(|e| e.to_string())?;
    let text = sample(&hir)?;
    if !regex_matches(pattern, &text) {
        return Err("Constructed witness does not satisfy regex".into());
    }
    Ok(text)
}

fn condition_witness(condition: &Condition, root: &mut Probe) -> Result<(), String> {
    match condition {
        Condition::All { children } => {
            for child in children {
                condition_witness(child, root)?;
            }
        }
        Condition::Any { children } => {
            condition_witness(children.first().ok_or("Empty OR")?, root)?
        }
        Condition::Body { pattern } => {
            if !root.body.is_empty() {
                root.body.push('\n');
            }
            root.body.push_str(&regex_witness(pattern)?);
        }
        Condition::Header { name, pattern } => root
            .headers
            .push((name.to_ascii_lowercase(), regex_witness(pattern)?)),
        Condition::Cookie { pattern } => root.set_cookie.push(regex_witness(pattern)?),
    }
    Ok(())
}

pub(crate) fn smoke_current(directory: &Path, text: &str) -> Result<serde_json::Value, String> {
    let candidates = checked_current_rules(text)?;
    let existing = combined_user_rules(directory)?;
    let prepared: std::collections::HashMap<_, _> = existing
        .iter()
        .map(|fp| (fp.id.clone(), serde_json::to_value(fp).unwrap()))
        .collect();
    for fp in &candidates {
        if prepared.get(&fp.id) != Some(&serde_json::to_value(fp).unwrap()) {
            return Err(format!("Prepared DATA_DIR differs for {}", fp.id));
        }
        if !fp.path_signals.is_empty() {
            return Err(format!("Smoke refuses path probes: {}", fp.id));
        }
    }
    let builtin = fingerprint_catalog();
    let loaded = all_fingerprints(Some(directory));
    let by_id: std::collections::HashMap<_, _> = loaded
        .iter()
        .map(|fp| (fp.id.clone(), serde_json::to_value(fp).unwrap()))
        .collect();
    let expected: std::collections::HashMap<_, _> = builtin
        .iter()
        .chain(existing.iter())
        .map(|fp| (fp.id.clone(), serde_json::to_value(fp).unwrap()))
        .collect();
    if by_id != expected || loaded.len() != expected.len() {
        return Err(
            "Runtime merge differs from defaults plus user overrides or contains duplicate IDs"
                .into(),
        );
    }
    let builtin_preserved = builtin
        .iter()
        .filter(|fp| by_id.get(&fp.id) == Some(&serde_json::to_value(fp).unwrap()))
        .count();
    let client = build_client(false).ok_or("Cannot construct runtime HTTP client")?;
    let negative = Probe {
        status: 200,
        headers: Vec::new(),
        set_cookie: Vec::new(),
        body: String::new(),
    };
    let mut cases = Vec::with_capacity(candidates.len());
    for fp in &candidates {
        if fingerprint_matches(&client, "http://127.0.0.1:9", fp, &negative) {
            return Err(format!("Empty response wrongly identifies {}", fp.id));
        }
        let mut positive = Probe {
            status: 200,
            headers: Vec::new(),
            set_cookie: Vec::new(),
            body: String::new(),
        };
        let mut generated = false;
        let mut last_error = String::new();
        if let Some(condition) = &fp.condition {
            condition_witness(condition, &mut positive)?;
            generated = true;
        }
        for (name, pattern) in &fp.header_regex {
            match regex_witness(pattern) {
                Ok(value) => {
                    positive.headers.push((name.to_ascii_lowercase(), value));
                    generated = true;
                    break;
                }
                Err(error) => last_error = error,
            }
        }
        if !generated {
            for pattern in &fp.set_cookie_regex {
                match regex_witness(pattern) {
                    Ok(value) => {
                        positive.set_cookie.push(value);
                        generated = true;
                        break;
                    }
                    Err(error) => last_error = error,
                }
            }
        }
        if !generated {
            for pattern in &fp.body_regex {
                match regex_witness(pattern) {
                    Ok(value) => {
                        positive.body = value;
                        generated = true;
                        break;
                    }
                    Err(error) => last_error = error,
                }
            }
        }
        if !generated {
            return Err(format!(
                "Cannot generate a positive root fixture for {}: {last_error}",
                fp.id
            ));
        }
        if !fingerprint_matches(&client, "http://127.0.0.1:9", fp, &positive) {
            return Err(format!("Runtime positive fixture failed for {}", fp.id));
        }
        cases.push(serde_json::json!({"id":fp.id,"positive":true,"emptyNegative":true}));
    }
    Ok(
        serde_json::json!({"valid":true,"sourceSha256":CURRENT_SOURCE_SHA256,"sourceFile":CURRENT_SOURCE_PATH,
        "runtimeSemantics":"legacy-any+condition-tree-v1","candidates":candidates.len(),"builtinPreserved":builtin_preserved,"effective":loaded.len(),
        "positiveCases":cases.len(),"negativeCases":cases.len(),"networkRequests":0,"databaseAccess":false,
        "snapshotScope":"exact current model/parser/loader/matcher functions; excludes Tauri and database recording",
        "cases":cases}),
    )
}

/// Evaluate inert synthetic responses through the exact current matcher; no paths.
pub(crate) fn audit_responses(text: &str, responses: &str) -> Result<serde_json::Value, String> {
    let rules = checked_current_rules(text)?;
    if rules.iter().any(|rule| !rule.path_signals.is_empty()) {
        return Err("Offline audit refuses path probes".into());
    }
    let fixtures: serde_json::Value = serde_json::from_str(responses).map_err(|e| e.to_string())?;
    let fixtures = fixtures
        .as_array()
        .ok_or("Expected synthetic response array")?;
    let known_ids: std::collections::HashSet<_> =
        rules.iter().map(|rule| rule.id.as_str()).collect();
    let client = build_client(false).ok_or("Cannot build inert matcher client")?;
    let mut results = Vec::new();
    for (index, fixture) in fixtures.iter().enumerate() {
        let fixture = fixture
            .as_object()
            .ok_or_else(|| format!("Response fixture {index} must be an object"))?;
        let expected_values = match fixture.get("expectedMatches") {
            Some(value) => value
                .as_array()
                .ok_or_else(|| format!("Fixture {index} expectedMatches must be an array"))?
                .as_slice(),
            None => &[],
        };
        let mut expected_matches = Vec::with_capacity(expected_values.len());
        let mut expected_ids = std::collections::HashSet::new();
        for value in expected_values {
            let id = value.as_str().ok_or_else(|| {
                format!("Fixture {index} expectedMatches must contain only rule ID strings")
            })?;
            if !known_ids.contains(id) {
                return Err(format!("Fixture {index} expects an unknown rule ID: {id}"));
            }
            if !expected_ids.insert(id) {
                return Err(format!("Fixture {index} repeats expected rule ID: {id}"));
            }
            expected_matches.push(id);
        }
        let headers: Vec<(String, String)> = fixture
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|values| {
                values
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.to_ascii_lowercase(),
                            v.as_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let response = Probe {
            status: fixture
                .get("status")
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as u16,
            headers,
            set_cookie: Vec::new(),
            body: fixture
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        };
        let matches: Vec<_> = rules
            .iter()
            .filter(|rule| fingerprint_matches(&client, "http://127.0.0.1:9", rule, &response))
            .map(|rule| rule.id.clone())
            .collect();
        let matched_ids: std::collections::HashSet<_> =
            matches.iter().map(String::as_str).collect();
        let unexpected_matches: Vec<_> = matches
            .iter()
            .filter(|id| !expected_ids.contains(id.as_str()))
            .cloned()
            .collect();
        let missing_matches: Vec<_> = expected_matches
            .iter()
            .filter(|id| !matched_ids.contains(**id))
            .copied()
            .collect();
        results.push(serde_json::json!({
            "name":fixture.get("name"),
            "expectedMatches":expected_matches,
            "matches":matches,
            "unexpectedMatches":unexpected_matches,
            "missingMatches":missing_matches
        }));
    }
    let valid = results.iter().all(|result| {
        ["unexpectedMatches", "missingMatches"].iter().all(|key| {
            result[*key]
                .as_array()
                .is_some_and(|matches| matches.is_empty())
        })
    });
    Ok(
        serde_json::json!({"valid":valid,"rules":rules.len(),"fixtures":fixtures.len(),"results":results,"networkRequests":0,"databaseAccess":false,"sourceSha256":CURRENT_SOURCE_SHA256}),
    )
}

/// Exercise the actual production bulk importer in a fresh synthetic directory.
pub(crate) fn fresh_import_smoke(
    directory: &Path,
    text: &str,
) -> Result<serde_json::Value, String> {
    let original_builtin: Vec<_> = fingerprint_catalog()
        .iter()
        .map(|r| serde_json::to_value(r).unwrap())
        .collect();
    let read_optional = |path: std::path::PathBuf| -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    };
    let first = merge_import(directory, text)?;
    let legacy = read_optional(user_rules_path(directory))?;
    let boolean = read_optional(boolean_rules_path(directory))?;
    let repeated = merge_import(directory, text)?;
    if first.len() != repeated.len()
        || read_optional(user_rules_path(directory))? != legacy
        || read_optional(boolean_rules_path(directory))? != boolean
    {
        return Err("Repeated full import rewrote existing rule data".into());
    }
    let loaded = all_fingerprints(Some(directory));
    let by_id: std::collections::HashMap<_, _> = loaded
        .iter()
        .map(|r| (r.id.clone(), serde_json::to_value(r).unwrap()))
        .collect();
    for original in &original_builtin {
        let id = original["id"].as_str().ok_or("Missing builtin ID")?;
        if by_id.get(id) != Some(original) {
            return Err("Full import changed builtin behavior".into());
        }
    }
    Ok(
        serde_json::json!({"valid":true,"legacy":read_user_fingerprints(directory)?.len(),"boolean":read_boolean_fingerprints(directory)?.len(),"effective":loaded.len(),"builtinPreserved":original_builtin.len(),"repeatBytesUnchanged":true,"localFilesCreated":usize::from(legacy.is_some())+usize::from(boolean.is_some()),"networkRequests":0,"realDatabaseAccess":false,"sourceSha256":CURRENT_SOURCE_SHA256}),
    )
}
