fn source_language(extension: &str) -> Option<&'static str> {
    match extension {
        "rs" => Some("Rust"),
        "ts" | "tsx" => Some("TypeScript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("JavaScript"),
        "vue" => Some("Vue SFC"),
        "java" => Some("Java"),
        "kt" | "kts" => Some("Kotlin"),
        "py" => Some("Python"),
        "php" => Some("PHP"),
        "go" => Some("Go"),
        "cs" => Some("C#"),
        "c" | "h" => Some("C"),
        "cc" | "cpp" | "cxx" | "hpp" => Some("C++"),
        "swift" => Some("Swift"),
        "rb" => Some("Ruby"),
        "scala" => Some("Scala"),
        "dart" => Some("Dart"),
        "sol" => Some("Solidity"),
        "html" | "htm" => Some("HTML"),
        "css" | "scss" | "sass" | "less" => Some("CSS"),
        "sql" => Some("SQL"),
        "sh" | "bash" | "zsh" => Some("Shell"),
        _ => None,
    }
}

fn source_line_counts(path: &Path, language: &str) -> Option<(u64, u64, u64, u64)> {
    let text = fs::read_to_string(path).ok()?;
    if text.as_bytes().contains(&0) {
        return None;
    }
    let mut physical = 0;
    let mut code = 0;
    let mut comments = 0;
    let mut blank = 0;
    let mut in_block_comment = false;
    for line in text.lines() {
        physical += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank += 1;
            continue;
        }
        if in_block_comment {
            comments += 1;
            if trimmed.contains("*/") || trimmed.contains("-->") {
                in_block_comment = false;
            }
            continue;
        }
        let line_comment = match language {
            "Python" | "Ruby" | "Shell" => trimmed.starts_with('#'),
            "SQL" => trimmed.starts_with("--"),
            "HTML" | "Vue SFC" => trimmed.starts_with("<!--"),
            _ => trimmed.starts_with("//"),
        };
        let block_comment =
            trimmed.starts_with("/*") || trimmed.starts_with('*') || trimmed.starts_with("<!--");
        if line_comment || block_comment {
            comments += 1;
            if (trimmed.starts_with("/*") && !trimmed.contains("*/"))
                || (trimmed.starts_with("<!--") && !trimmed.contains("-->"))
            {
                in_block_comment = true;
            }
        } else {
            code += 1;
            if let Some(start) = trimmed.find("/*") {
                if !trimmed[start + 2..].contains("*/") {
                    in_block_comment = true;
                }
            }
        }
    }
    Some((physical, code, comments, blank))
}

fn inspect_source_tree(root: &Path) -> JsonValue {
#[allow(clippy::too_many_arguments)]
    fn visit(
        path: &Path,
        root: &Path,
        depth: usize,
        counts: &mut HashMap<String, [u64; 6]>,
        manifests: &mut Vec<String>,
        samples: &mut Vec<PathBuf>,
        total: &mut u64,
        skipped_large: &mut u64,
    ) {
        if depth > 24 || *total >= 100_000 {
            return;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            if *total >= 100_000 {
                break;
            }
            // Never follow links into a different source tree (or a link cycle).
            // An unknown file type is not evidence that this path is in scope.
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let child = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if file_type.is_dir() {
                if [
                    ".git",
                    "node_modules",
                    "target",
                    "dist",
                    "build",
                    "vendor",
                    ".venv",
                    "venv",
                    "coverage",
                    ".next",
                ]
                .contains(&name.as_str())
                {
                    continue;
                }
                visit(
                    &child,
                    root,
                    depth + 1,
                    counts,
                    manifests,
                    samples,
                    total,
                    skipped_large,
                );
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            *total += 1;
            let relative = child
                .strip_prefix(root)
                .unwrap_or(&child)
                .to_string_lossy()
                .replace('\\', "/");
            if [
                "package.json",
                "Cargo.toml",
                "tauri.conf.json",
                "pom.xml",
                "build.gradle",
                "build.gradle.kts",
                "requirements.txt",
                "pyproject.toml",
                "composer.json",
                "go.mod",
                "Gemfile",
            ]
            .contains(&name.as_str())
            {
                manifests.push(relative);
                samples.push(child.clone());
            }
            let extension = child
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if let Some(language) = source_language(&extension) {
                let size = entry.metadata().map(|v| v.len()).unwrap_or(0);
                let value = counts.entry(language.into()).or_default();
                value[0] += 1;
                value[1] += size;
                if size > 5 * 1024 * 1024 {
                    *skipped_large += 1;
                } else if let Some((lines, code, comments, blank)) =
                    source_line_counts(&child, language)
                {
                    value[2] += lines;
                    value[3] += code;
                    value[4] += comments;
                    value[5] += blank;
                }
            }
        }
    }
    let mut counts = HashMap::new();
    let mut manifests = Vec::new();
    let mut samples = Vec::new();
    let mut total = 0;
    let mut skipped_large = 0;
    visit(
        root,
        root,
        0,
        &mut counts,
        &mut manifests,
        &mut samples,
        &mut total,
        &mut skipped_large,
    );
    let code_files = counts.values().map(|value| value[0]).sum::<u64>();
    let total_lines = counts.values().map(|value| value[2]).sum::<u64>();
    let code_lines = counts.values().map(|value| value[3]).sum::<u64>();
    let comment_lines = counts.values().map(|value| value[4]).sum::<u64>();
    let blank_lines = counts.values().map(|value| value[5]).sum::<u64>();
    let mut languages=counts.into_iter().map(|(name,value)|serde_json::json!({"name":name,"files":value[0],"bytes":value[1],"lines":value[2],"codeLines":value[3],"commentLines":value[4],"blankLines":value[5],"percent":if code_files==0{0.0}else{value[0] as f64*100.0/code_files as f64}})).collect::<Vec<_>>();
    languages.sort_by(|a, b| {
        b.get("files")
            .and_then(JsonValue::as_u64)
            .cmp(&a.get("files").and_then(JsonValue::as_u64))
    });
    let mut manifest_documents = Vec::new();
    for (manifest, path) in manifests.iter().zip(samples.iter()).take(30) {
        if let Ok(text) = fs::read_to_string(path) {
            if text.len() < 2_000_000 {
                manifest_documents.push((manifest.clone(), text.to_ascii_lowercase()));
            }
        }
    }
    let mut frameworks = Vec::new();
    let mut add = |name: &str, layer: &str, needle: &str| {
        if let Some((manifest, _)) = manifest_documents
            .iter()
            .find(|(_, content)| content.contains(needle))
        {
            frameworks.push(serde_json::json!({"name":name,"layer":layer,"evidence":manifest}))
        }
    };
    add("Tauri", "Desktop runtime", "tauri");
    add("Vue", "Frontend", "\"vue\"");
    add("React", "Frontend", "\"react\"");
    add("Angular", "Frontend", "@angular/core");
    add("Next.js", "Frontend", "\"next\"");
    add("Vite", "Build", "\"vite\"");
    add("Spring", "Backend", "spring-boot");
    add("Django", "Backend", "django");
    add("Flask", "Backend", "flask");
    add("FastAPI", "Backend", "fastapi");
    add("Laravel", "Backend", "laravel/framework");
    add("Symfony", "Backend", "symfony/framework");
    add("Gin", "Backend", "github.com/gin-gonic/gin");
    add("Fiber", "Backend", "github.com/gofiber/fiber");
    add("Axum", "Backend", "axum");
    add("Actix Web", "Backend", "actix-web");
    let architecture = if frameworks
        .iter()
        .any(|v| v.get("name").and_then(JsonValue::as_str) == Some("Tauri"))
    {
        "Tauri desktop application"
    } else if frameworks
        .iter()
        .any(|v| v.get("layer").and_then(JsonValue::as_str) == Some("Frontend"))
    {
        "Web application"
    } else {
        "Source repository"
    };
    serde_json::json!({"architecture":architecture,"root":root,"totalFiles":total,"codeFiles":code_files,"lineStats":{"physical":total_lines,"code":code_lines,"comments":comment_lines,"blank":blank_lines,"skippedLargeFiles":skipped_large,"maxFileBytes":5*1024*1024},"languages":languages,"frameworks":frameworks,"manifests":manifests,"detectedAt":chrono::Utc::now().to_rfc3339()})
}

fn insert_source_inventory(
    connection: &rusqlite::Connection,
    scan_id: &str,
    source_path: &str,
) -> Result<(), String> {
    if source_path.is_empty() {
        return Ok(());
    }
    let inventory = inspect_source_tree(Path::new(source_path));
    insert_finding(
        connection,
        scan_id,
        source_path,
        "local-inventory",
        "source_inventory",
        "repository",
        "源码架构与语言清单",
        "info",
        &inventory,
    )
}
