//! Validated, passive product-identification condition trees.
//! Legacy flat signal rules keep their original matching path in exposure_probe.
use serde::{Deserialize, Serialize};

pub(crate) const BOOLEAN_RUNTIME: &str = "condition-tree-v1";
const MAX_DEPTH: usize = 16;
const MAX_NODES: usize = 256;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum Condition {
    All { children: Vec<Condition> },
    Any { children: Vec<Condition> },
    Body { pattern: String },
    Header { name: String, pattern: String },
    Cookie { pattern: String },
}

fn validate_pattern(pattern: &str) -> Result<(), String> {
    if pattern.is_empty() || pattern.len() > 4096 {
        return Err("条件正则必须为 1–4096 字节".into());
    }
    let compiled = regex::RegexBuilder::new(pattern)
        .size_limit(2 * 1024 * 1024)
        .dfa_size_limit(2 * 1024 * 1024)
        .nest_limit(64)
        .build()
        .map_err(|error| format!("条件正则无效：{error}"))?;
    if compiled.is_match("") {
        return Err("条件正则不能匹配空字符串".into());
    }
    Ok(())
}

pub(crate) fn validate_condition(condition: &Condition) -> Result<(), String> {
    fn visit(condition: &Condition, depth: usize, nodes: &mut usize) -> Result<(), String> {
        *nodes += 1;
        if depth > MAX_DEPTH || *nodes > MAX_NODES {
            return Err("条件树超过 16 层或 256 个节点上限".into());
        }
        match condition {
            Condition::All { children } | Condition::Any { children } => {
                if children.is_empty() {
                    return Err("AND/OR 条件组不能为空".into());
                }
                for child in children {
                    visit(child, depth + 1, nodes)?;
                }
                Ok(())
            }
            Condition::Body { pattern } | Condition::Cookie { pattern } => {
                validate_pattern(pattern)
            }
            Condition::Header { name, pattern } => {
                if name.is_empty()
                    || name.len() > 128
                    || !name.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
                    })
                {
                    return Err("条件 Header 名称无效".into());
                }
                validate_pattern(pattern)
            }
        }
    }
    visit(condition, 1, &mut 0)
}

pub(crate) fn condition_matches(
    condition: &Condition,
    body: &str,
    headers: &[(String, String)],
    cookies: &[String],
) -> bool {
    fn matches(pattern: &str, value: &str) -> bool {
        regex::Regex::new(pattern).is_ok_and(|compiled| compiled.is_match(value))
    }
    match condition {
        Condition::All { children } => {
            !children.is_empty()
                && children
                    .iter()
                    .all(|child| condition_matches(child, body, headers, cookies))
        }
        Condition::Any { children } => children
            .iter()
            .any(|child| condition_matches(child, body, headers, cookies)),
        Condition::Body { pattern } => matches(pattern, body),
        Condition::Header { name, pattern } => headers
            .iter()
            .any(|(header, value)| header.eq_ignore_ascii_case(name) && matches(pattern, value)),
        Condition::Cookie { pattern } => cookies.iter().any(|cookie| matches(pattern, cookie)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(value: serde_json::Value) -> Condition {
        serde_json::from_value(value).unwrap()
    }
    #[test]
    fn and_requires_every_child_and_nested_or_keeps_its_group() {
        let tree = parse(serde_json::json!({"type":"all","children":[
            {"type":"body","pattern":"ProductA"},
            {"type":"any","children":[{"type":"header","name":"server","pattern":"ServerB"},{"type":"cookie","pattern":"SESSION_C="}]}
        ]}));
        validate_condition(&tree).unwrap();
        assert!(!condition_matches(&tree, "ProductA", &[], &[]));
        assert!(!condition_matches(
            &tree,
            "Login",
            &[("Server".into(), "ServerB".into())],
            &[]
        ));
        assert!(condition_matches(
            &tree,
            "ProductA",
            &[("SERVER".into(), "ServerB".into())],
            &[]
        ));
        assert!(condition_matches(
            &tree,
            "ProductA",
            &[],
            &["SESSION_C=1".into()]
        ));
        assert!(!condition_matches(
            &tree,
            "<html>公告 帮助中心 404 Login</html>",
            &[],
            &[]
        ));
    }
    #[test]
    fn or_does_not_drop_a_mandatory_condition_inside_one_branch() {
        let tree = parse(serde_json::json!({"type":"any","children":[
            {"type":"all","children":[{"type":"body","pattern":"BrandA"},{"type":"body","pattern":"ModuleA"}]},
            {"type":"all","children":[{"type":"body","pattern":"BrandB"},{"type":"header","name":"x-brand","pattern":"B"}]}
        ]}));
        assert!(!condition_matches(&tree, "BrandA", &[], &[]));
        assert!(!condition_matches(&tree, "BrandB", &[], &[]));
        assert!(condition_matches(&tree, "BrandA ModuleA", &[], &[]));
        assert!(condition_matches(
            &tree,
            "BrandB",
            &[("x-brand".into(), "B".into())],
            &[]
        ));
    }
    #[test]
    fn invalid_groups_patterns_and_headers_are_rejected() {
        for value in [
            serde_json::json!({"type":"all","children":[]}),
            serde_json::json!({"type":"any","children":[]}),
            serde_json::json!({"type":"body","pattern":".*"}),
            serde_json::json!({"type":"body","pattern":"("}),
            serde_json::json!({"type":"header","name":"Server\r\nX","pattern":"app"}),
        ] {
            assert!(validate_condition(&parse(value)).is_err());
        }
        assert!(!condition_matches(
            &Condition::All { children: vec![] },
            "",
            &[],
            &[]
        ));
        assert!(serde_json::from_value::<Condition>(
            serde_json::json!({"type":"not","children":[]})
        )
        .is_err());
        assert!(serde_json::from_value::<Condition>(
            serde_json::json!({"type":"body","pattern":"app","cve":"CVE-2026-0000"})
        )
        .is_err());
    }
    #[test]
    fn recursive_and_node_limits_are_enforced() {
        let mut tree = Condition::Body {
            pattern: "Product".into(),
        };
        for _ in 0..16 {
            tree = Condition::All {
                children: vec![tree],
            };
        }
        assert!(validate_condition(&tree).is_err());
        assert!(validate_condition(&Condition::Any {
            children: (0..256)
                .map(|_| Condition::Body {
                    pattern: "Product".into()
                })
                .collect()
        })
        .is_err());
    }
}
