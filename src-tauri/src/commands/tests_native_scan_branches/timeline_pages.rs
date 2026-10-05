#[test]
fn timeline_history_pages_are_bounded_lossless_and_attempt_fenced() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    for index in 0..230 {
        connection.execute(
            "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status) \
             VALUES(?1,'source-regression',1,'https://authorized.example.test','native','spa_api_mapper','prepared')",
            [format!("page-run-{index:03}")],
        ).unwrap();
    }
    let before_reads = connection.total_changes();
    let latest = native_scan_status(&connection, "source-regression").unwrap();
    assert_eq!(latest["timeline"].as_array().unwrap().len(), 100);
    assert_eq!(latest["hasEarlierTimeline"], true);
    let mut seen = std::collections::HashSet::new();
    for item in latest["timeline"].as_array().unwrap() {
        assert!(seen.insert(item["id"].as_str().unwrap().to_string()));
    }
    let mut cursor = latest["timelineBeforeSequence"].as_i64().unwrap();
    while cursor > 0 {
        let page = native_scan_timeline_page(&connection, "source-regression", 1, cursor).unwrap();
        let items = page["timeline"].as_array().unwrap();
        assert!(items.len() <= 100);
        for item in items {
            assert!(seen.insert(item["id"].as_str().unwrap().to_string()), "duplicate item: {item}");
        }
        if page["hasEarlierTimeline"] == false { break; }
        let next = page["timelineBeforeSequence"].as_i64().unwrap();
        assert!(next > 0 && next < cursor, "history must advance backwards");
        cursor = next;
    }
    for index in 0..230 {
        assert!(seen.contains(&format!("page-run-{index:03}")));
    }
    assert_eq!(connection.total_changes(), before_reads, "history reads are not mailbox acknowledgements");
    assert!(native_scan_timeline_page(&connection, "source-regression", 2, cursor).is_err());
    assert!(native_scan_timeline_page(&connection, "source-regression", 1, i64::MAX).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn incremental_timeline_replays_large_bursts_without_skipping_a_watermark() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    let cursor = native_scan_status(&connection, "source-regression").unwrap()["latestSequence"].as_i64().unwrap();
    for index in 0..215 {
        connection.execute(
            "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,status) \
             VALUES(?1,'source-regression',1,'https://authorized.example.test','native','spa_api_mapper','prepared')",
            [format!("burst-run-{index:03}")],
        ).unwrap();
    }
    let mut next = cursor;
    let mut seen = std::collections::HashSet::new();
    for expected_count in [100, 100, 15] {
        let page = native_scan_status_after(&connection, "source-regression", Some(next)).unwrap();
        assert_eq!(page["isIncremental"], true);
        assert_eq!(page["timeline"].as_array().unwrap().len(), expected_count);
        for item in page["timeline"].as_array().unwrap() {
            assert!(seen.insert(item["id"].as_str().unwrap().to_string()));
        }
        let watermark = page["latestSequence"].as_i64().unwrap();
        assert!(watermark > next);
        next = watermark;
    }
    assert_eq!(seen.len(), 215);
    assert!(native_scan_status_after(&connection, "source-regression", Some(next)).unwrap()
        ["timeline"].as_array().unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}
