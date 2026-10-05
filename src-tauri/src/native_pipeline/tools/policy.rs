//! Model tool arguments are rejected before dispatch to any source capability.

/// Argument keys that are never accepted, whatever the tool. Each one is the shape of a
/// capability §10.2 item 6 takes away from the model.
pub(super) fn forbidden_key(key: &str) -> Option<(&'static str, &'static str)> {
    let lowered = key.to_ascii_lowercase();
    let shell = [
        "command", "cmd", "shell", "script", "exec", "execute", "eval", "argv",
    ];
    let binary = [
        "binary",
        "program",
        "executable",
        "image",
        "analyzer",
        "analyzerpath",
        "enginepath",
        "toolpath",
    ];
    let rules = [
        "rules",
        "ruleurl",
        "rulespath",
        "rulepack",
        "config",
        "configpath",
        "queries",
    ];
    let network = [
        "url", "uri", "network", "http", "fetch", "download", "host", "proxy",
    ];
    let write = [
        "write", "content", "contents", "patch", "apply", "delete", "remove", "move", "rename",
        "mkdir", "chmod", "append",
    ];
    if shell.contains(&lowered.as_str()) {
        return Some(("arbitrary_shell", "模型不能请求执行命令"));
    }
    if binary.contains(&lowered.as_str()) {
        return Some((
            "self_chosen_binary",
            "分析器二进制和镜像由计划固定，模型不能自选",
        ));
    }
    if rules.contains(&lowered.as_str()) {
        return Some((
            "self_downloaded_rules",
            "规则包只能来自冻结的计划，模型不能指定或下载",
        ));
    }
    if network.contains(&lowered.as_str()) {
        return Some(("unapproved_network", "源码工具不联网，网络访问必须另行授权"));
    }
    if write.contains(&lowered.as_str()) {
        return Some(("source_modification", "源码只读，任何写入都会被拒绝"));
    }
    None
}

/// Keys each tool actually reads. Anything else is refused before a handler runs, so a
/// plausible-looking extra field cannot smuggle in a capability.
pub(super) fn allowed_keys(name: &str) -> &'static [&'static str] {
    match name {
        "repo.inventory" => &["prefix", "limit"],
        "repo.search" => &["query", "prefix", "limit", "ignoreCase"],
        "repo.read_slice" => &["path", "startLine", "maxLines"],
        "git.changed_files" => &[],
        "analyzer.list_results" => &["engine", "limit"],
        "analyzer.get_result" => &["key"],
        "callgraph.get_slice" => &["path", "symbol"],
        "dependency.get_record" => &["manifest", "name"],
        "evidence.submit_candidate" => &[
            "title",
            "severity",
            "rule",
            "path",
            "line",
            "rationale",
            "sourceAnalyzer",
            "cwe",
        ],
        "assignment.finish" => &["summary", "gaps"],
        _ => &[],
    }
}

pub(super) fn path_like(name: &str) -> bool {
    matches!(name, "path" | "prefix" | "manifest")
}
