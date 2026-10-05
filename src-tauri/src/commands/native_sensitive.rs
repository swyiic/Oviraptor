//! Deterministic match + exclusion/semantic passes. A pattern hit is evidence,
//! not proof that a credential is active or a vulnerability is exploitable.
use super::*;

fn pack(path: &Path, fallback: &str) -> JsonValue {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| serde_json::from_str(fallback).expect("bundled rule pack"))
}

fn strings<'a>(value: &'a JsonValue, key: &str) -> impl Iterator<Item = &'a str> {
    value
        .get(key)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
}

fn excluded(
    rule: &JsonValue,
    exclusions: &[JsonValue],
    context: &str,
    value: &str,
    kind: &str,
) -> bool {
    let haystack = format!("{context} {value}");
    let lower = haystack.to_lowercase();
    let required = strings(rule, "requireContext").collect::<Vec<_>>();
    if !required.is_empty() && !required.iter().any(|v| lower.contains(&v.to_lowercase())) {
        return true;
    }
    std::iter::once(rule)
        .chain(exclusions.iter().filter(|item| {
            let applies = value_first(item, &["appliesToKind"]);
            applies == "*"
                || applies.eq_ignore_ascii_case(kind)
                || strings(item, "appliesToKinds").any(|v| v.eq_ignore_ascii_case(kind))
        }))
        .any(|item| {
            strings(item, "excludeSignals").any(|signal| lower.contains(&signal.to_lowercase()))
                || strings(item, "excludePatterns").any(|pattern| {
                    regex::Regex::new(pattern).is_ok_and(|re| re.is_match(&haystack))
                })
        })
}

fn valid_identity_number(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 18 || !bytes[..17].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let number = |start, end| {
        value
            .get(start..end)
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or_default()
    };
    if chrono::NaiveDate::from_ymd_opt(number(6, 10) as i32, number(10, 12), number(12, 14))
        .is_none()
    {
        return false;
    }
    let weights = [7, 9, 10, 5, 8, 4, 2, 1, 6, 3, 7, 9, 10, 5, 8, 4, 2];
    let sum = bytes[..17]
        .iter()
        .zip(weights)
        .map(|(value, weight)| (value - b'0') as usize * weight)
        .sum::<usize>();
    b"10X98765432"[sum % 11] == bytes[17].to_ascii_uppercase()
}

fn plausible_secret(value: &str) -> bool {
    let lower = value.to_lowercase();
    if value.chars().count() < 8
        || [
            "password",
            "passwd",
            "undefined",
            "null",
            "example",
            "changeme",
            "12345678",
        ]
        .contains(&lower.as_str())
    {
        return false;
    }
    if ["${", "{{", "}}", "<%", "%>"]
        .iter()
        .any(|s| value.contains(s))
        || value.starts_with('+')
        || value.ends_with('+')
    {
        return false;
    }
    static REFERENCE: OnceLock<regex::Regex> = OnceLock::new();
    if REFERENCE
        .get_or_init(|| {
            regex::Regex::new(r"^(?:this\.)?[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*|\[[^\]]+\])+$")
                .unwrap()
        })
        .is_match(value)
    {
        return false;
    }
    value.chars().any(|c| c.is_ascii_alphabetic())
        && value.chars().any(|c| !c.is_ascii_alphabetic())
}

pub(super) fn scan(source: &str, source_url: &str, rules_path: &Path) -> Vec<JsonValue> {
    let rules = pack(
        rules_path,
        include_str!("../../resources/rules/sensitive.json"),
    );
    let exclusions = pack(
        &rules_path.with_file_name("exclusions.json"),
        include_str!("../../resources/rules/exclusions.json"),
    );
    let exclusions = exclusions
        .get("exclusions")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    for rule in rules
        .get("rules")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        if value_first(rule, &["channel"]) != "sensitive" {
            continue;
        }
        let kind = value_first(rule, &["type", "kind"]);
        let raw_pattern = value_first(rule, &["pattern"]);
        // Rust's linear regex engine has no lookarounds. These two bundled
        // numeric guards are checked explicitly below, without broadening hits.
        let pattern = raw_pattern
            .replace("(?<![0-9])", "")
            .replace("(?![0-9])", "");
        let Ok(expression) = regex::Regex::new(&pattern) else {
            continue;
        };
        for capture in expression.captures_iter(source).take(64) {
            let whole = capture.get(0).expect("whole match");
            if matches!(kind.as_str(), "cn_phone" | "cn_id")
                && (source
                    .as_bytes()
                    .get(whole.start().wrapping_sub(1))
                    .is_some_and(u8::is_ascii_digit)
                    || source
                        .as_bytes()
                        .get(whole.end())
                        .is_some_and(u8::is_ascii_digit))
            {
                continue;
            }
            let group = rule
                .get("valueGroup")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0) as usize;
            let value = capture.get(group).unwrap_or(whole).as_str();
            let before = source[..whole.start()]
                .chars()
                .rev()
                .take(180)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            let after = source[whole.end()..].chars().take(180).collect::<String>();
            let context = format!("{before}{}{after}", whole.as_str());
            let lower = context.to_lowercase();
            if excluded(rule, &exclusions, &context, value, &kind) {
                continue;
            }
            if matches!(
                kind.as_str(),
                "hardcoded_credential"
                    | "bearer_token"
                    | "aws_secret_access_key"
                    | "oauth_client_secret"
            ) && !plausible_secret(value)
            {
                continue;
            }
            if kind == "cn_id" && !valid_identity_number(value) {
                continue;
            }
            if kind == "cn_phone"
                && ![
                    "phone",
                    "mobile",
                    "telephone",
                    "tel",
                    "contact",
                    "手机",
                    "电话",
                    "联系方式",
                ]
                .iter()
                .any(|s| lower.contains(s))
            {
                continue;
            }
            if kind == "email"
                && [
                    "copyright",
                    "license",
                    "@preserve",
                    "contributors",
                    "package.json",
                    "npmjs",
                ]
                .iter()
                .any(|s| lower.contains(s))
            {
                continue;
            }
            let mut scope = "";
            if kind == "ip_address" {
                let Ok(ip) = value.parse::<std::net::Ipv4Addr>() else {
                    continue;
                };
                if ["0.0.0.0", "1.2.3.4", "127.0.0.1", "255.255.255.255"].contains(&value) {
                    continue;
                }
                if [
                    "<path",
                    "viewbox",
                    "pathdata",
                    "svgpath",
                    "iconpath",
                    "version",
                    "sourcemappingurl",
                ]
                .iter()
                .any(|s| lower.contains(s))
                {
                    continue;
                }
                let adjacent = |index: usize| {
                    source
                        .as_bytes()
                        .get(index)
                        .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
                };
                if adjacent(whole.start().wrapping_sub(1)) || adjacent(whole.end()) {
                    continue;
                }
                let private = ip.is_private() || ip.is_link_local() || ip.is_loopback();
                if !private
                    && ![
                        "http://",
                        "https://",
                        "ws://",
                        "host",
                        "server",
                        "proxy",
                        "endpoint",
                        "address",
                        "服务器",
                        "地址",
                    ]
                    .iter()
                    .any(|s| lower.contains(s))
                {
                    continue;
                }
                scope = if private { "private" } else { "public" };
            }
            let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
            if !seen.insert(format!("{kind}|{digest}")) {
                continue;
            }
            let explicit_secret = [
                "secret",
                "credential",
                "password",
                "passwd",
                "private_key",
                "token",
                "authorization",
                "密钥",
                "密码",
                "令牌",
                "生产",
            ]
            .iter()
            .any(|s| lower.contains(s));
            let noise_source = [
                "node_modules",
                "/vendor/",
                "/example",
                "/fixture",
                "/test/",
                "/docs/",
                "readme",
                "license",
                "sample",
            ]
            .iter()
            .any(|s| format!("{source_url} {lower}").to_lowercase().contains(s));
            let public_identifier = kind == "firebase_database_url";
            let noise = noise_source && !explicit_secret;
            let classification = if noise {
                "noise"
            } else if public_identifier {
                "public_identifier"
            } else {
                "probable"
            };
            let severity = if noise {
                "info".to_string()
            } else {
                value_first(rule, &["severity"])
            };
            let mask = format!(
                "{}…{}",
                value.chars().take(4).collect::<String>(),
                value
                    .chars()
                    .rev()
                    .take(3)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<String>()
            );
            records.push(serde_json::json!({
                "type":kind,"label":value_first(rule,&["label"]),"severity":severity,
                "confidence":if noise {"low"} else {"medium"},"classification":classification,
                "source":source_url,"value":value,"maskedValue":mask,"sha256":digest,"scope":scope,
                "evidence":whole.as_str().chars().take(180).collect::<String>(),
                "context":context.chars().take(400).collect::<String>(),
                "ruleId":value_first(rule,&["id"]),"ruleSource":"resources/rules/sensitive.json",
                "reviewPasses":["pattern_match","exclusion_and_semantic_review"],"requiresVerification":true
            }));
        }
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scan_fixture(source: &str) -> Vec<JsonValue> {
        scan(
            source,
            "https://fixture.test/app.js",
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/rules/sensitive.json"),
        )
    }
    #[test]
    fn excludes_runtime_references_and_placeholder_secrets() {
        assert!(
            scan_fixture("password:this.form.password; token:example1234; secret:changeme")
                .is_empty()
        );
    }
    #[test]
    fn preserves_two_pass_fields_and_does_not_claim_confirmed_vulnerability() {
        let rows = scan_fixture("const password='fixture-82q7aKey';");
        assert!(!rows.is_empty());
        assert_eq!(rows[0]["reviewPasses"].as_array().unwrap().len(), 2);
        assert_eq!(rows[0]["classification"], "probable");
        assert_eq!(rows[0]["value"], "fixture-82q7aKey");
    }
    #[test]
    fn checks_phone_numeric_boundaries_and_id_checksum() {
        assert!(scan_fixture("mobile:0138001380009; id:110105194912310021").is_empty());
        assert!(scan_fixture("mobile:13800138000")
            .iter()
            .any(|v| v["type"] == "cn_phone"));
    }
    #[test]
    fn all_bundled_patterns_are_supported() {
        let rules: JsonValue =
            serde_json::from_str(include_str!("../../resources/rules/sensitive.json")).unwrap();
        for rule in rules["rules"].as_array().unwrap() {
            let pattern = value_first(rule, &["pattern"])
                .replace("(?<![0-9])", "")
                .replace("(?![0-9])", "");
            assert!(
                regex::Regex::new(&pattern).is_ok(),
                "unsupported rule {}",
                rule["id"]
            );
        }
    }
}
