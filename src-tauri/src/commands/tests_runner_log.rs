// Runner-log reads (§3): the attempt row decides which directory is read, and a
// missing file, an empty file and an unreadable file stay three different answers.
    #[test]
    fn runner_log_live_notification_redacts_and_bounds_the_display_payload() {
        let payload = runner_log_notification("native-log", 2,
            "\x1b[32mretry with password=live-secret\x1b[0m");
        assert_eq!(payload["scanId"], "native-log");
        assert_eq!(payload["attempt"], 2);
        let line = payload["line"].as_str().unwrap();
        assert!(line.starts_with("retry with password=<redacted:auth:"));
        assert!(!payload.to_string().contains("live-secret"));
        assert!(!line.contains('\x1b'));
        assert_eq!(payload.as_object().unwrap().len(), 3);
        let long = runner_log_notification("native-log", 2, &"界".repeat(1400));
        assert_eq!(long["line"].as_str().unwrap().chars().count(), 1200);
    }

    #[test]
    fn runner_log_live_and_tail_views_match_without_rewriting_evidence() {
        let root = std::env::temp_dir().join(format!("oviraptor-runner-display-{}", Uuid::new_v4()));
        let dir = root.join("agent-jobs/native-log/attempt-0001");
        let raw = "\x1b[32mretry with password=stored-secret\x1b[0m";
        runner_log_write(&dir, raw);
        let path = dir.join("oviraptor-runner.log");
        let tail = read_runner_log_tail(&path, 10);
        let payload = runner_log_notification("native-log", 1, raw);
        assert_eq!(tail.lines, vec![payload["line"].as_str().unwrap().to_string()]);
        assert!(!tail.lines[0].contains("stored-secret"));
        assert_eq!(fs::read_to_string(path).unwrap(), raw);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runner_log_identity_reads_the_attempt_directory() {
        let path = Path::new("/tmp/agent-jobs/517af51a-68c3-49e8-9774-b0778158a93d/attempt-0002/oviraptor-runner.log");
        assert_eq!(
            runner_log_identity(path),
            Some(("517af51a-68c3-49e8-9774-b0778158a93d".into(), 2))
        );
        assert!(runner_log_identity(Path::new("/tmp/notes.txt")).is_none());
    }

    fn runner_log_db(root: &Path, scan_id: &str) -> PathBuf {
        let db_path = db::initialize(root).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(7201,'Runner log')", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) \
                 VALUES(?1,7201,'Runner log','failed','web')",
                [scan_id],
            )
            .unwrap();
        drop(connection);
        db_path
    }

    fn runner_log_add_scan(db_path: &Path, scan_id: &str) {
        let connection = db::open(db_path).unwrap();
        connection
            .execute(
                "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) \
                 VALUES(?1,7201,'Runner log','failed','web')",
                [scan_id],
            )
            .unwrap();
    }

    fn runner_log_store_attempt(
        db_path: &Path,
        scan_id: &str,
        attempt: i64,
        status: &str,
        stage: &str,
        work_dir: &Path,
    ) {
        let connection = db::open(db_path).unwrap();
        connection
            .execute(
                "INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status,stage,work_dir) \
                 VALUES(?1,?2,?3,?4,?5)",
                params![scan_id, attempt, status, stage, work_dir.to_string_lossy()],
            )
            .unwrap();
    }

    fn runner_log_write(dir: &Path, text: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("oviraptor-runner.log"), text).unwrap();
    }

    #[test]
    fn runner_log_reads_the_attempt_directory_recorded_in_the_database() {
        let root = std::env::temp_dir().join(format!("oviraptor-runner-log-{}", Uuid::new_v4()));
        let scan_id = "runner-log-attempt";
        let db_path = runner_log_db(&root, scan_id);
        // The older attempt owns the lexicographically later directory, so a reader
        // that sorted directory names instead of trusting work_dir picks the wrong one.
        let first_dir = root.join("strix-jobs").join(scan_id).join("attempt-z");
        let latest_dir = root.join("strix-jobs").join(scan_id).join("attempt-a");
        runner_log_write(&first_dir, "first attempt line");
        runner_log_write(&latest_dir, "latest attempt line");
        runner_log_store_attempt(&db_path, scan_id, 1, "failed", "running", &first_dir);
        runner_log_store_attempt(&db_path, scan_id, 2, "stopped", "stopping", &latest_dir);

        let view = read_sentinel_runner_log_inner(&db_path, &root, scan_id, None, None).unwrap();
        assert_eq!(view.attempt, 2);
        assert_eq!(view.status, "ready");
        assert_eq!(view.source, "attempt_work_dir");
        assert_eq!(view.attempt_status, "stopped");
        assert_eq!(view.stage, "stopping");
        assert_eq!(view.lines, vec!["latest attempt line".to_string()]);
        assert_eq!(view.message, "");

        let first =
            read_sentinel_runner_log_inner(&db_path, &root, scan_id, Some(1), None).unwrap();
        assert_eq!(first.attempt, 1);
        assert_eq!(first.lines, vec!["first attempt line".to_string()]);
        assert_eq!(
            get_sentinel_runner_log_inner(&db_path, &root, scan_id.to_string(), None).unwrap(),
            vec!["latest attempt line".to_string()]
        );
        let missing = read_sentinel_runner_log_inner(&db_path, &root, scan_id, Some(9), None);
        assert!(missing.unwrap_err().contains("第 9 次执行"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runner_log_keeps_missing_empty_and_unrecorded_apart() {
        let root = std::env::temp_dir().join(format!("oviraptor-runner-log-{}", Uuid::new_v4()));
        let scan_id = "runner-log-states";
        let db_path = runner_log_db(&root, scan_id);
        let no_attempt_scan = "runner-log-no-attempts";
        runner_log_add_scan(&db_path, no_attempt_scan);

        let absent_dir = root.join("strix-jobs").join(scan_id).join("attempt-absent");
        fs::create_dir_all(&absent_dir).unwrap();
        runner_log_store_attempt(&db_path, scan_id, 1, "failed", "running", &absent_dir);
        let absent = read_sentinel_runner_log_inner(&db_path, &root, scan_id, None, None).unwrap();
        assert_eq!(absent.status, "not_created");
        assert!(absent.lines.is_empty());
        assert!(absent.message.contains("尚未生成"), "{}", absent.message);

        let empty_dir = root.join("strix-jobs").join(scan_id).join("attempt-empty");
        runner_log_write(&empty_dir, "\n   \n");
        runner_log_store_attempt(&db_path, scan_id, 2, "failed", "running", &empty_dir);
        let empty = read_sentinel_runner_log_inner(&db_path, &root, scan_id, None, None).unwrap();
        assert_eq!(empty.status, "empty");
        assert_eq!(empty.attempt, 2);
        assert!(empty.message.contains("没有可显示"), "{}", empty.message);

        let old_dir = root.join("strix-jobs").join(no_attempt_scan);
        runner_log_write(&old_dir, "old directory must not be read");
        let fallback =
            read_sentinel_runner_log_inner(&db_path, &root, no_attempt_scan, None, None).unwrap();
        assert_eq!(fallback.attempt, 0);
        assert_eq!(fallback.source, "task_root");
        assert_eq!(fallback.status, "not_created");
        assert!(fallback.lines.is_empty());
        assert!(
            fallback.message.contains("没有执行记录"),
            "{}",
            fallback.message
        );
        let current_dir = root.join("agent-jobs").join(no_attempt_scan);
        runner_log_write(&current_dir, "current task line");
        let filled =
            read_sentinel_runner_log_inner(&db_path, &root, no_attempt_scan, None, None).unwrap();
        assert_eq!(filled.status, "ready");
        assert_eq!(filled.lines, vec!["current task line".to_string()]);
        assert!(read_sentinel_runner_log_inner(&db_path, &root, "missing-scan", None, None)
            .unwrap_err()
            .contains("任务不存在"));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn runner_log_reports_an_unreadable_file_as_a_read_failure() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("oviraptor-runner-log-{}", Uuid::new_v4()));
        let scan_id = "runner-log-denied";
        let db_path = runner_log_db(&root, scan_id);
        let dir = root.join("strix-jobs").join(scan_id).join("attempt-denied");
        runner_log_write(&dir, "hidden line");
        fs::set_permissions(
            dir.join("oviraptor-runner.log"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        runner_log_store_attempt(&db_path, scan_id, 1, "failed", "running", &dir);
        let denied = read_sentinel_runner_log_inner(&db_path, &root, scan_id, None, None).unwrap();
        assert_eq!(denied.status, "read_failed");
        assert!(denied.message.contains("日志读取失败"), "{}", denied.message);
        fs::set_permissions(
            dir.join("oviraptor-runner.log"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let readable =
            read_sentinel_runner_log_inner(&db_path, &root, scan_id, None, None).unwrap();
        assert_eq!(readable.status, "ready");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runner_log_keeps_the_previous_tail_limit_and_line_cleaning() {
        let root = std::env::temp_dir().join(format!("oviraptor-runner-log-{}", Uuid::new_v4()));
        let scan_id = "runner-log-tail";
        let db_path = runner_log_db(&root, scan_id);
        let dir = root.join("strix-jobs").join(scan_id).join("attempt-tail");
        fs::create_dir_all(&dir).unwrap();
        let long_line = format!("{}tail-marker", "x".repeat(1400));
        fs::write(
            dir.join("oviraptor-runner.log"),
            format!("line 1\nline 2\nline 3\nline 4\nline 5\n\x1b[32mcolored line\x1b[0m\nretry with password=sup3r-secret\n{long_line}\n"),
        )
        .unwrap();
        runner_log_store_attempt(&db_path, scan_id, 1, "failed", "running", &dir);
        let view = read_sentinel_runner_log_inner(&db_path, &root, scan_id, None, Some(4)).unwrap();
        assert_eq!(
            view.lines.len(),
            4,
            "the tail must keep the last lines, in order: {:?}",
            view.lines
        );
        assert_eq!(view.lines[0], "line 5");
        // ANSI escapes are stripped before the line is shown.
        assert_eq!(view.lines[1], "colored line");
        // The credential value is replaced by a stable marker; the digest is not
        // asserted, only that the secret itself never reaches the UI.
        assert!(
            view.lines[2].starts_with("retry with password=<redacted:auth:"),
            "{}",
            view.lines[2]
        );
        assert!(!view.lines.iter().any(|line| line.contains("sup3r-secret")));
        assert_eq!(view.lines[3], "x".repeat(1200));
        let _ = fs::remove_dir_all(root);
    }
