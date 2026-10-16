//! The same intentionally finite schema subset as the JavaScript SDK v2.
use serde_json::Value;

pub(crate) fn object_keys(value: &Value, allowed: &[&str], required: &[&str]) -> bool {
    value.as_object().is_some_and(|o| {
        o.keys().all(|k| allowed.contains(&k.as_str()))
            && required.iter().all(|k| o.contains_key(*k))
    })
}

pub(crate) fn bytes(value: &Value, max: usize) -> Result<Vec<u8>, String> {
    struct Limited(Vec<u8>, usize);
    impl std::io::Write for Limited {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            if data.len() > self.1.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("budget"));
            }
            self.0.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut out = Limited(Vec::new(), max);
    serde_json::to_writer(&mut out, value).map_err(|_| "app_result_too_large")?;
    Ok(out.0)
}

fn nonnegative(v: &Value, max: u64) -> bool {
    v.as_u64().is_some_and(|n| n <= max)
}
fn scalar_type(kind: &str, v: &Value) -> bool {
    match kind {
        "string" => v.is_string(),
        "integer" => v
            .as_i64()
            .is_some_and(|n| n.unsigned_abs() <= crate::MAX_SAFE_INTEGER as u64),
        "number" => v.as_f64().is_some_and(f64::is_finite),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        _ => false,
    }
}

fn schema_type(schema: &Value, action: bool) -> Result<(&str, bool), String> {
    let value = schema.get("type").ok_or("app_schema_invalid")?;
    if let Some(kind) = value.as_str() {
        return Ok((kind, false));
    }
    if action {
        if let Some(types) = value.as_array().filter(|types| types.len() == 2) {
            let kinds: Vec<_> = types.iter().filter_map(Value::as_str).collect();
            if kinds.len() == 2
                && kinds.contains(&"null")
                && kinds.iter().filter(|kind| **kind == "null").count() == 1
            {
                return Ok((
                    kinds.into_iter().find(|kind| *kind != "null").unwrap(),
                    true,
                ));
            }
        }
    }
    Err("app_schema_invalid".into())
}

pub(crate) fn validate(schema: &Value, action: bool, depth: usize) -> Result<(), String> {
    let fail = || "app_schema_invalid".to_string();
    let (kind, nullable) = schema_type(schema, action)?;
    let extra: &[&str] = match kind {
        "object" => &["properties", "required", "additionalProperties"],
        "array" => &["items", "minItems", "maxItems"],
        "string" => &["minLength", "maxLength"],
        "integer" | "number" => &["minimum", "maximum"],
        "boolean" | "null" => &[],
        _ => return Err(fail()),
    };
    let allowed: Vec<&str> = ["type", "description", "enum"]
        .into_iter()
        .chain(extra.iter().copied())
        .collect();
    if depth > 8
        || !object_keys(schema, &allowed, &["type"])
        || schema
            .get("description")
            .is_some_and(|v| !v.as_str().is_some_and(|s| s.encode_utf16().count() <= 1024))
    {
        return Err(fail());
    }
    if let Some(values) = schema.get("enum") {
        let values = values.as_array().ok_or_else(fail)?;
        if values.is_empty()
            || values.len() > 32
            || matches!(kind, "object" | "array")
            || values
                .iter()
                .any(|v| !(nullable && v.is_null()) && !scalar_type(kind, v))
            || values
                .iter()
                .enumerate()
                .any(|(i, v)| values[..i].contains(v))
            || bytes(&Value::Array(values.clone()), 4096).is_err()
        {
            return Err(fail());
        }
    }
    match kind {
        "object" => {
            let props = schema
                .get("properties")
                .and_then(Value::as_object)
                .ok_or_else(fail)?;
            if props.len() > if action { 256 } else { 64 }
                || schema.get("additionalProperties") != Some(&Value::Bool(false))
            {
                return Err(fail());
            }
            for (name, sub) in props {
                if name.is_empty()
                    || name.encode_utf16().count() > 128
                    || matches!(name.as_str(), "__proto__" | "prototype" | "constructor")
                {
                    return Err(fail());
                }
                validate(sub, action, depth + 1)?;
            }
            if let Some(required) = schema.get("required") {
                let required = required.as_array().ok_or_else(fail)?;
                if required.len() > if action { 256 } else { 64 }
                    || required
                        .iter()
                        .any(|v| !v.as_str().is_some_and(|k| props.contains_key(k)))
                    || required
                        .iter()
                        .enumerate()
                        .any(|(i, v)| required[..i].contains(v))
                {
                    return Err(fail());
                }
            }
        }
        "array" => {
            let max = schema
                .get("maxItems")
                .filter(|v| nonnegative(v, if action { 16384 } else { 128 }))
                .and_then(Value::as_u64)
                .ok_or_else(fail)?;
            if schema.get("minItems").is_some_and(|v| !nonnegative(v, max)) {
                return Err(fail());
            }
            validate(schema.get("items").ok_or_else(fail)?, action, depth + 1)?;
        }
        "string" => {
            let cap = if action { 1024 * 1024 } else { 16384 };
            if schema
                .get("maxLength")
                .is_some_and(|v| !nonnegative(v, cap))
            {
                return Err(fail());
            }
            let max = schema
                .get("maxLength")
                .and_then(Value::as_u64)
                .unwrap_or(cap);
            if schema
                .get("minLength")
                .is_some_and(|v| !nonnegative(v, max))
            {
                return Err(fail());
            }
        }
        "integer" | "number" => {
            for field in ["minimum", "maximum"] {
                if schema.get(field).is_some_and(|v| !scalar_type(kind, v)) {
                    return Err(fail());
                }
            }
            if let (Some(min), Some(max)) = (
                schema.get("minimum").and_then(Value::as_f64),
                schema.get("maximum").and_then(Value::as_f64),
            ) {
                if min > max {
                    return Err(fail());
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn arguments(schema: &Value, value: &Value, action: bool) -> Result<(), String> {
    validate(schema, action, 0)?;
    bytes(value, if action { 1024 * 1024 } else { 64 * 1024 })
        .map_err(|_| "app_arguments_invalid")?;
    fn visit(s: &Value, v: &Value, action: bool) -> Result<(), String> {
        let (kind, nullable) = schema_type(s, action)?;
        if nullable && v.is_null() {
            if s.get("enum")
                .is_some_and(|e| !e.as_array().is_some_and(|a| a.contains(v)))
            {
                return Err("app_arguments_invalid".into());
            }
            return Ok(());
        }
        let valid = match kind {
            "object" => v.is_object(),
            "array" => v.is_array(),
            _ => scalar_type(kind, v),
        };
        if !valid
            || s.get("enum")
                .is_some_and(|e| !e.as_array().is_some_and(|a| a.contains(v)))
        {
            return Err("app_arguments_invalid".into());
        }
        match kind {
            "object" => {
                let props = s["properties"].as_object().ok_or("app_schema_invalid")?;
                let obj = v.as_object().ok_or("app_arguments_invalid")?;
                if obj.keys().any(|k| !props.contains_key(k))
                    || s.get("required")
                        .and_then(Value::as_array)
                        .is_some_and(|r| {
                            r.iter()
                                .any(|k| !obj.contains_key(k.as_str().unwrap_or("")))
                        })
                {
                    return Err("app_arguments_invalid".into());
                }
                for (k, x) in obj {
                    visit(&props[k], x, action)?;
                }
            }
            "array" => {
                let a = v.as_array().ok_or("app_arguments_invalid")?;
                if a.len() < s.get("minItems").and_then(Value::as_u64).unwrap_or(0) as usize
                    || a.len() > s["maxItems"].as_u64().ok_or("app_schema_invalid")? as usize
                {
                    return Err("app_arguments_invalid".into());
                }
                for x in a {
                    visit(&s["items"], x, action)?;
                }
            }
            "string" => {
                let n = v.as_str().ok_or("app_arguments_invalid")?.chars().count() as u64;
                if n < s.get("minLength").and_then(Value::as_u64).unwrap_or(0)
                    || n > s
                        .get("maxLength")
                        .and_then(Value::as_u64)
                        .unwrap_or(if action { 1024 * 1024 } else { 16384 })
                {
                    return Err("app_arguments_invalid".into());
                }
            }
            "integer" | "number" => {
                let n = v.as_f64().ok_or("app_arguments_invalid")?;
                if s.get("minimum")
                    .and_then(Value::as_f64)
                    .is_some_and(|m| n < m)
                    || s.get("maximum")
                        .and_then(Value::as_f64)
                        .is_some_and(|m| n > m)
                {
                    return Err("app_arguments_invalid".into());
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(schema, value, action)
}
