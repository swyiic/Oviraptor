// Stage 1 (§12): the frozen legacy artifact fixtures (§9.1) and the read-only hard
// boundary (§9.2). These tests never import from the repository copy, so a broken
// importer cannot dirty the committed evidence.
    const FROZEN_MTIME_SECONDS: u64 = 1_577_836_800; // 2020-01-01T00:00:00Z

    pub(super) fn legacy_fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy_strix")
    }

    fn sha256_hex(path: &Path) -> String {
        let digest = Sha256::digest(fs::read(path).expect("fixture unreadable"));
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    }

    fn relative_key(root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("fixture directory unreadable") {
            let path = entry.expect("fixture entry").path();
            if path.is_dir() {
                walk_files(&path, out);
            } else {
                out.push(path);
            }
        }
    }

    /// `(relative path -> (sha256, bytes))` for every fixture except the manifest.
    fn fixture_inventory(root: &Path) -> std::collections::BTreeMap<String, (String, u64)> {
        let mut files = Vec::new();
        walk_files(root, &mut files);
        let mut inventory = std::collections::BTreeMap::new();
        for path in files {
            let key = relative_key(root, &path);
            if matches!(key.as_str(), "manifest.json" | "residual_baseline.json") {
                continue;
            }
            inventory.insert(
                key,
                (
                    sha256_hex(&path),
                    fs::metadata(&path).map(|value| value.len()).unwrap_or(0),
                ),
            );
        }
        inventory
    }

    /// `(relative path -> fingerprint)` where the fingerprint binds content, length,
    /// permission bits and modification time.
    fn directory_fingerprint(root: &Path) -> std::collections::BTreeMap<String, String> {
        let mut files = Vec::new();
        walk_files(root, &mut files);
        let mut rows = std::collections::BTreeMap::new();
        for path in files {
            let metadata = fs::metadata(&path).expect("fixture metadata");
            let modified = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| format!("{}", value.as_nanos()))
                .unwrap_or_else(|| "unknown".to_string());
            #[cfg(unix)]
            let mode = format!("{:o}", {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o7777
            });
            #[cfg(not(unix))]
            let mode = if metadata.permissions().readonly() {
                "ro"
            } else {
                "rw"
            };
            rows.insert(
                relative_key(root, &path),
                format!(
                    "{}|{}|{}|{}",
                    sha256_hex(&path),
                    metadata.len(),
                    mode,
                    modified
                ),
            );
        }
        rows
    }

    /// Copies the fixture bundles into a temp tree with a fixed mtime and mode, so a
    /// writer inside the importer is visible as a fingerprint change.
    pub(super) fn copy_tree_frozen_times(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for path in {
            let mut files = Vec::new();
            walk_files(source, &mut files);
            files
        } {
            let target = destination.join(relative_key(source, &path));
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(&path, &target).expect("fixture copy");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = fs::metadata(&path)
                    .map(|value| value.permissions().mode() & 0o7777)
                    .unwrap_or(0o644);
                let _ = fs::set_permissions(&target, fs::Permissions::from_mode(mode));
            }
            let times = std::fs::FileTimes::new()
                .set_accessed(std::time::SystemTime::UNIX_EPOCH
                    + std::time::Duration::from_secs(FROZEN_MTIME_SECONDS))
                .set_modified(std::time::SystemTime::UNIX_EPOCH
                    + std::time::Duration::from_secs(FROZEN_MTIME_SECONDS));
            std::fs::File::options()
                .write(true)
                .open(&target)
                .expect("open for times")
                .set_times(times)
                .expect("fixture mtime must be settable");
        }
    }

    #[test]
    fn legacy_fixtures_match_their_frozen_manifest() {
        let root = legacy_fixture_root();
        let manifest_path = root.join("manifest.json");
        let manifest: JsonValue =
            serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest missing"))
                .expect("manifest JSON");
        let frozen = manifest
            .get("files")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let inventory = fixture_inventory(&root);
        assert!(
            inventory.len() >= 20,
            "冻结集必须覆盖 §9.1 列出的全部格式，当前只有 {} 个文件",
            inventory.len()
        );
        for (key, (digest, bytes)) in &inventory {
            let entry = frozen
                .get(key)
                .unwrap_or_else(|| panic!("冻结清单缺少文件 {key}"));
            assert_eq!(&value_first(entry, &["sha256"]), digest, "内容漂移：{key}");
            assert_eq!(
                entry.get("bytes").and_then(JsonValue::as_u64),
                Some(*bytes),
                "长度漂移：{key}"
            );
        }
        for key in frozen.keys() {
            assert!(
                inventory.contains_key(key),
                "冻结清单登记了不存在的文件 {key}"
            );
        }
    }

    #[test]
    fn legacy_fixture_inventory_tracks_retirement_inputs_not_supported_formats() {
        let inventory = fixture_inventory(&legacy_fixture_root());
        let paths = inventory.keys().cloned().collect::<Vec<_>>();
        let contains = |needle: &str| paths.iter().any(|path| path.contains(needle));
        // 冻结集是待退役输入清单，不是支持格式清单；负向用例仍使用其中的旧文件。
        for (needle, label) in [
            ("producer_1_5_3", "1.5.3 run.json"),
            ("producer_1_6_2", "1.6.2 run.json"),
            ("vulnerabilities.json", "顶层 array"),
            ("envelope_vulnerabilities", "vulnerabilities envelope"),
            ("envelope_findings", "findings envelope"),
            ("envelope_results", "results envelope"),
            ("envelope_items", "items envelope"),
            ("findings.sarif", "SARIF"),
            ("vulnerabilities.csv", "CSV"),
            ("vulnerabilities/", "Markdown"),
            ("coverage.json", "coverage schema 1"),
            ("events.jsonl", "events JSONL"),
            ("events.ndjson", "events NDJSON"),
            ("oviraptor_recon.json", "旧 recon"),
            ("asset_atlas_recon.json", "更早 recon"),
            ("s1_", "S1"),
            ("s2_", "S2"),
            ("s3_", "S3"),
            ("s4_", "S4"),
            ("s5_", "S5"),
            ("summary.json", "summary"),
            ("meta.json", "meta"),
        ] {
            assert!(contains(needle), "冻结集缺少 {label}（{needle}）");
        }
    }

    /// Imports a copy of the frozen bundles. Returns how many bundles were claimed.
    /// Binds each bundle to a scan before any snapshot is taken: the importer itself
    /// must never add, rename or rewrite anything in the source tree (§9.2).
    pub(super) fn claim_legacy_bundles(source: &Path) {
        for run in ["producer_1_5_3", "producer_1_6_2"] {
            let dir = source.join(run);
            assert!(dir.is_dir(), "冻结集缺少 bundle {run}");
            fs::write(dir.join(".asset-atlas-scan-id"), "legacy-frozen").unwrap();
        }
    }

    pub(super) fn import_legacy_bundles(source: &Path) -> Result<i64, String> {
        let root = source.parent().unwrap().to_path_buf();
        let app_dir = root.join("oviraptor");
        let db_path = db::initialize(&app_dir).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(8101,'Legacy Freeze')", [])
            .unwrap();
        connection.execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,scan_type,attempt_count) \
             VALUES('legacy-frozen',8101,'Legacy Freeze','completed','已完成','web',1)",
            [],
        )
        .unwrap();
        drop(connection);
        let connection = db::open(&db_path).unwrap();
        artifact_import_report(&connection, &app_dir.join("artifact-cas"),
            &app_dir.join("artifact-import.key"), &[source.to_path_buf()]).map(|report| report.imported as i64)
    }

    #[test]
    fn importing_legacy_artifacts_leaves_the_source_directory_untouched() {
        let root = std::env::temp_dir().join(format!("oviraptor-legacy-frozen-{}", Uuid::new_v4()));
        let source = root.join("source-bundles");
        copy_tree_frozen_times(&legacy_fixture_root().join("bundles"), &source);
        claim_legacy_bundles(&source);
        let before = directory_fingerprint(&source);
        assert!(before.len() >= 20, "快照文件数异常：{}", before.len());
        let synced = import_legacy_bundles(&source).expect("历史结果导入必须成功");
        let after = directory_fingerprint(&source);
        assert!(
            synced >= 1,
            "导入没有认领任何 bundle，快照对比就成了空断言"
        );
        assert_eq!(before, after, "§9.2：导入器不得写、删、改名或 chmod 源目录");
        let _ = fs::remove_dir_all(root);
    }
