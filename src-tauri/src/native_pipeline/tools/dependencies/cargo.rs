//! Cargo declaration tables only; version requirements are never resolved versions.
use serde_json::{json, Value};

fn append_tables(owner: &toml::Value, rows: &mut Vec<Value>) -> Result<(), &'static str> {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(value) = owner.get(section) else {
            continue;
        };
        let table = value.as_table().ok_or("cargo_dependency_table_invalid")?;
        for (name, declaration) in table {
            if name.is_empty() {
                return Err("cargo_dependency_name_invalid");
            }
            let requirement = match declaration {
                toml::Value::String(version) => version.as_str(),
                toml::Value::Table(fields) => match fields.get("version") {
                    Some(value) => value.as_str().ok_or("cargo_dependency_version_invalid")?,
                    None if fields.get("workspace").and_then(toml::Value::as_bool)
                        == Some(true)
                        || fields
                            .get("path")
                            .and_then(toml::Value::as_str)
                            .is_some_and(|s| !s.is_empty())
                        || fields
                            .get("git")
                            .and_then(toml::Value::as_str)
                            .is_some_and(|s| !s.is_empty()) =>
                    {
                        ""
                    }
                    None => return Err("cargo_dependency_source_unresolved"),
                },
                _ => return Err("cargo_dependency_declaration_invalid"),
            };
            rows.push(json!({"name":name,"requirement":requirement,"scope":"rust"}));
        }
    }
    Ok(())
}

pub(super) fn parse(text: &str) -> Result<Vec<Value>, &'static str> {
    let document: toml::Value = toml::from_str(text).map_err(|_| "cargo_manifest_invalid")?;
    let mut rows = Vec::new();
    append_tables(&document, &mut rows)?;
    if let Some(workspace) = document.get("workspace") {
        workspace
            .as_table()
            .ok_or("cargo_workspace_table_invalid")?;
        append_tables(workspace, &mut rows)?;
    }
    if let Some(targets) = document.get("target") {
        for (_, target) in targets.as_table().ok_or("cargo_target_table_invalid")? {
            target.as_table().ok_or("cargo_target_table_invalid")?;
            append_tables(target, &mut rows)?;
        }
    }
    // Keep all declarations: target-specific requirements may disagree.
    rows.sort_by_key(|row| {
        (
            row["name"].as_str().unwrap_or_default().to_string(),
            row["requirement"].as_str().unwrap_or_default().to_string(),
        )
    });
    Ok(rows)
}
