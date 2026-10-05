//! Frozen declaration-only dependency records; no network resolution.

use super::{BrokerDenial, SourceBroker};
use serde_json::{json, Value as JsonValue};

mod cargo;
mod gradle;

impl SourceBroker {
    pub(super) fn dependency_record(
        &self,
        map: &serde_json::Map<String, JsonValue>,
    ) -> Result<JsonValue, BrokerDenial> {
        let manifest = Self::normalized(
            map.get("manifest")
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
        );
        if manifest.is_empty() {
            return Err(BrokerDenial::new(
                "invalid_arguments",
                format!(
                    "dependency.get_record 需要 manifest，本仓库有：{}",
                    self.selected_manifests().join(", ")
                ),
            ));
        }
        let text = self.read_frozen(&manifest)?;
        let wanted = map
            .get("name")
            .and_then(JsonValue::as_str)
            .map(str::to_string);
        let name = manifest.rsplit('/').next().unwrap_or(&manifest);
        let entries = match name {
            "Cargo.toml" => cargo::parse(&text),
            "build.gradle" | "build.gradle.kts" => gradle::parse(&text),
            _ => Ok(parse_dependencies(&manifest, &text)),
        }
        .map_err(|reason| BrokerDenial::new("dependency_declaration_invalid", reason))?;
        let filtered: Vec<JsonValue> = entries
            .into_iter()
            .filter(|row| {
                wanted
                    .as_deref()
                    .is_none_or(|name| row.get("name").and_then(JsonValue::as_str) == Some(name))
            })
            .collect();
        Ok(json!({
            "manifest": manifest,
            "contentHash": self.selected_hash(&manifest).unwrap_or_default(),
            "dependencies": filtered,
            "note": "只覆盖声明清单，传递依赖需要 lockfile 或 SBOM",
        }))
    }
}

/// The dependency declarations this manifest format actually expresses.
fn parse_dependencies(manifest: &str, text: &str) -> Vec<JsonValue> {
    let name = manifest.rsplit('/').next().unwrap_or(manifest);
    match name {
        "package.json" => serde_json::from_str::<JsonValue>(text)
            .ok()
            .map(|parsed| {
                ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"]
                    .iter()
                    .filter_map(|section| parsed.get(*section))
                    .filter_map(JsonValue::as_object)
                    .flat_map(|map| {
                        map.iter().map(|(key, value)| {
                            json!({"name": key, "requirement": value.as_str().unwrap_or_default(), "scope": "npm"})
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "requirements.txt" | "requirements-dev.txt" => text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('-'))
            .map(|line| {
                let split = line.find(['=', '<', '>', '!', '~', '[']).unwrap_or(line.len());
                let (name, requirement) = line.split_at(split);
                json!({"name": name.trim(), "requirement": requirement.trim(), "scope": "python"})
            })
            .collect(),
        "go.mod" => text
            .lines()
            .map(str::trim)
            .filter_map(|line| {
                let rest = line.strip_prefix("require ").unwrap_or(line);
                let (name, version) = rest.split_once(char::is_whitespace)?;
                let version = version
                    .trim()
                    .trim_start_matches('(')
                    .trim_end_matches(')');
                // A module path holds a dot or a slash; a version always starts with `v`.
                if !(name.contains('.') || name.contains('/')) || !version.starts_with('v') {
                    return None;
                }
                Some(json!({"name": name, "requirement": version, "scope": "go"}))
            })
            .collect(),
        "pom.xml" => {
            // `<artifactId>name</artifactId>` followed by its `<version>`: good enough
            // for the declarations a gate needs, and never a guess about transitives.
            let mut rows = Vec::new();
            let mut cursor = 0usize;
            while let Some(found) = text[cursor..].find("<artifactId>") {
                let start = cursor + found + "<artifactId>".len();
                let Some(end) = text[start..].find("</artifactId>") else {
                    break;
                };
                let name = text[start..start + end].trim().to_string();
                let tail = &text[start + end..];
                let version = tail
                    .find("<version>")
                    .and_then(|at| {
                        let from = at + "<version>".len();
                        tail[from..].find("</version>").map(|stop| {
                            tail[from..from + stop].trim().to_string()
                        })
                    })
                    .unwrap_or_default();
                rows.push(json!({"name": name, "requirement": version, "scope": "java"}));
                cursor = start + end;
            }
            rows
        }
        _ => Vec::new(),
    }
}
