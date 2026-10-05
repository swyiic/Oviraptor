/// One rejected call: a stable message, the JSON paths that broke, and a
/// fingerprint so the loop can tell "the same mistake again" from progress (§7).
struct ArgumentRejection {
    message: String,
    errors: Vec<JsonValue>,
    fingerprint: String,
}

/// Which keyword position holds a nested schema.
#[derive(Default)]
enum ExtraProperties {
    /// `additionalProperties: false` — unknown keys are a violation.
    Disallow,
    /// Not declared.
    #[default]
    Allow,
    /// `additionalProperties: {…}` — every value must match this schema.
    Checked(Box<CompiledSchema>),
}

/// A parameter schema compiled once at first use (§7). Everything the model may
/// pass is described here: types, required keys, closed objects, array items,
/// enums, constants, numeric and string bounds, collection sizes and URL formats.
#[derive(Default)]
struct CompiledSchema {
    types: Vec<&'static str>,
    required: Vec<String>,
    properties: Vec<(String, CompiledSchema)>,
    additional: ExtraProperties,
    items: Option<Box<CompiledSchema>>,
    enumerated: Option<Vec<JsonValue>>,
    constant: Option<JsonValue>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    min_length: Option<usize>,
    max_length: Option<usize>,
    min_items: Option<usize>,
    max_items: Option<usize>,
    url_format: bool,
}

fn static_type_name(name: &str) -> Option<&'static str> {
    match name {
        "object" => Some("object"),
        "array" => Some("array"),
        "string" => Some("string"),
        "number" => Some("number"),
        "integer" => Some("integer"),
        "boolean" => Some("boolean"),
        "null" => Some("null"),
        _ => None,
    }
}

fn compile_schema(schema: &JsonValue) -> CompiledSchema {
    let number = |key: &str| schema.get(key).and_then(JsonValue::as_f64);
    let size = |key: &str| number(key).map(|value| value.max(0.0) as usize);
    let types: Vec<&'static str> = match schema.get("type") {
        Some(JsonValue::String(name)) => static_type_name(name).into_iter().collect(),
        Some(JsonValue::Array(rows)) => rows
            .iter()
            .filter_map(JsonValue::as_str)
            .filter_map(static_type_name)
            .collect(),
        _ => Vec::new(),
    };
    CompiledSchema {
        types,
        required: schema
            .get("required")
            .and_then(JsonValue::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(JsonValue::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        properties: schema
            .get("properties")
            .and_then(JsonValue::as_object)
            .map(|map| {
                map.iter()
                    .map(|(key, value)| (key.clone(), compile_schema(value)))
                    .collect()
            })
            .unwrap_or_default(),
        additional: match schema.get("additionalProperties") {
            Some(JsonValue::Bool(false)) | Some(JsonValue::Null) => ExtraProperties::Disallow,
            Some(JsonValue::Bool(true)) => ExtraProperties::Allow,
            Some(value) => ExtraProperties::Checked(Box::new(compile_schema(value))),
            None => ExtraProperties::Allow,
        },
        items: schema
            .get("items")
            .filter(|value| !value.is_array())
            .map(|value| Box::new(compile_schema(value))),
        enumerated: schema.get("enum").and_then(JsonValue::as_array).cloned(),
        constant: schema.get("const").cloned(),
        minimum: number("minimum"),
        maximum: number("maximum"),
        min_length: size("minLength"),
        max_length: size("maxLength"),
        min_items: size("minItems"),
        max_items: size("maxItems"),
        url_format: schema.get("format").and_then(JsonValue::as_str) == Some("url"),
    }
}

struct SchemaViolation {
    path: String,
    keyword: &'static str,
    expected: String,
    actual: String,
}

impl SchemaViolation {
    fn to_json(&self) -> JsonValue {
        serde_json::json!({
            "path": self.path,
            "keyword": self.keyword,
            "expected": self.expected,
            "actual": self.actual,
        })
    }
}

fn json_type_name(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
        JsonValue::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
    }
}

fn type_matches(declared: &str, value: &JsonValue) -> bool {
    match declared {
        "integer" => matches!(json_type_name(value), "integer"),
        "number" => matches!(json_type_name(value), "integer" | "number"),
        other => json_type_name(value) == other,
    }
}

fn actual_description(value: &JsonValue) -> String {
    match value {
        JsonValue::String(text) => format!("string({})", text.chars().take(40).collect::<String>()),
        other => format!("{}:{other}", json_type_name(other)),
    }
}

impl CompiledSchema {
    fn validate(&self, value: &JsonValue, path: &str, out: &mut Vec<SchemaViolation>) {
        let violation = |keyword: &'static str, expected: String, out: &mut Vec<SchemaViolation>| {
            out.push(SchemaViolation {
                path: path.to_string(),
                keyword,
                expected,
                actual: actual_description(value),
            })
        };
        if !self.types.is_empty()
            && !self.types.iter().any(|declared| type_matches(declared, value))
        {
            violation(
                "type",
                format!("one of [{}]", self.types.join(", ")),
                out,
            );
            // A wrong type has no meaningful nested checks.
            return;
        }
        if let Some(allowed) = &self.enumerated {
            if !allowed.contains(value) {
                violation("enum", allowed.iter().map(JsonValue::to_string).collect(), out);
            }
        }
        if let Some(constant) = &self.constant {
            if constant != value {
                violation("const", constant.to_string(), out);
            }
        }
        match value {
            JsonValue::String(text) => {
                let length = text.chars().count();
                if self.min_length.is_some_and(|min| length < min) {
                    violation("minLength", self.min_length.map(|min| min.to_string()).unwrap_or_default(), out);
                }
                if self.max_length.is_some_and(|max| length > max) {
                    violation("maxLength", self.max_length.map(|max| max.to_string()).unwrap_or_default(), out);
                }
                if self.url_format
                    && reqwest::Url::parse(text)
                        .map(|parsed| !matches!(parsed.scheme(), "http" | "https"))
                        .unwrap_or(true)
                {
                    violation("format", "url".to_string(), out);
                }
            }
            JsonValue::Number(number) => {
                let Some(value) = number.as_f64() else { return };
                if self.minimum.is_some_and(|min| value < min) {
                    violation("minimum", self.minimum.map(|min| min.to_string()).unwrap_or_default(), out);
                }
                if self.maximum.is_some_and(|max| value > max) {
                    violation("maximum", self.maximum.map(|max| max.to_string()).unwrap_or_default(), out);
                }
            }
            JsonValue::Array(rows) => {
                if self.min_items.is_some_and(|min| rows.len() < min) {
                    violation("minItems", self.min_items.map(|min| min.to_string()).unwrap_or_default(), out);
                }
                if self.max_items.is_some_and(|max| rows.len() > max) {
                    violation("maxItems", self.max_items.map(|max| max.to_string()).unwrap_or_default(), out);
                }
                if let Some(items) = &self.items {
                    for (index, row) in rows.iter().enumerate() {
                        items.validate(row, &format!("{path}[{index}]"), out);
                    }
                }
            }
            JsonValue::Object(map) => {
                for key in &self.required {
                    if !map.contains_key(key) {
                        out.push(SchemaViolation {
                            path: format!("{path}.{key}"),
                            keyword: "required",
                            expected: "present".to_string(),
                            actual: "missing".to_string(),
                        });
                    }
                }
                for (key, item) in map {
                    let child_path = if path.is_empty() {
                        format!("$.{key}")
                    } else {
                        format!("{path}.{key}")
                    };
                    match self.properties.iter().find(|(name, _)| name == key) {
                        Some((_, schema)) => schema.validate(item, &child_path, out),
                        None => match &self.additional {
                            ExtraProperties::Disallow => out.push(SchemaViolation {
                                path: child_path,
                                keyword: "additionalProperties",
                                expected: "none".to_string(),
                                actual: key.clone(),
                            }),
                            ExtraProperties::Checked(schema) => {
                                schema.validate(item, &child_path, out)
                            }
                            ExtraProperties::Allow => {}
                        },
                    }
                }
            }
            _ => {}
        }
    }
}

fn agent_schema_registry() -> &'static HashMap<&'static str, CompiledSchema> {
    static REGISTRY: OnceLock<HashMap<&'static str, CompiledSchema>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        agent_tool_specs()
            .into_iter()
            .map(|spec| (spec.name, compile_schema(&spec.parameters)))
            .collect()
    })
}

/// Argument validation for every tool, against its full compiled schema. A
/// rejected call never reaches the target and never spends request budget (§7).
fn agent_validate_arguments(name: &str, arguments: &JsonValue) -> Result<(), ArgumentRejection> {
    let reject = |message: String, errors: Vec<JsonValue>| -> ArgumentRejection {
        let fingerprint = agent_stable_hash(&serde_json::json!({"name": name, "errors": errors}));
        ArgumentRejection {
            message,
            errors,
            fingerprint: fingerprint[..16].to_string(),
        }
    };
    if arguments.get("__parseError").is_some() {
        return Err(reject(
            "工具参数不是合法 JSON 对象".to_string(),
            vec![serde_json::json!({
                "path": "$", "keyword": "type", "expected": "object", "actual": "unparsed",
            })],
        ));
    }
    // An unknown tool has no schema at all: refuse before any dispatch.
    let Some(schema) = agent_schema_registry().get(name) else {
        return Err(reject(
            format!("未知工具：{name}"),
            vec![serde_json::json!({
                "path": "$", "keyword": "tool", "expected": "a registered tool name", "actual": name,
            })],
        ));
    };
    let mut violations = Vec::new();
    schema.validate(arguments, "$", &mut violations);
    if violations.is_empty() {
        return Ok(());
    }
    let first = &violations[0];
    Err(reject(
        format!(
            "{name} 参数不符合 schema：{} 处 {} 期望 {}，实际 {}",
            first.path, first.keyword, first.expected, first.actual
        ),
        violations
            .iter()
            .map(SchemaViolation::to_json)
            .take(8)
            .collect(),
    ))
}
