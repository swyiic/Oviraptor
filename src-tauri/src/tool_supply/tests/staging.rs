use super::*;
use crate::tool_supply::cache::{
    stage_offline_tool_candidate, stage_offline_tool_candidate_from_file,
};

#[test]
fn candidate_staging_is_content_addressed_and_revalidated() {
    let root = std::env::temp_dir().join(format!("oviraptor-tool-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let db_path = crate::db::initialize(&root).unwrap();
    let cache = root.join("tool-cache");
    let (mut manifest, anchor, package, keypair) = fixture();
    let raw = sign(&mut manifest, &keypair);
    let destination = stage_offline_tool_candidate(
        &db_path,
        &cache,
        &raw,
        &package,
        std::slice::from_ref(&anchor),
        "macos-aarch64",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for directory in [&cache, &cache.join("browser-runtime")] {
            assert_eq!(
                fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        assert_eq!(
            fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
            0o500
        );
    }
    assert_eq!(fs::read(destination.join("package.blob")).unwrap(), package);
    assert_eq!(
        crate::db::environment_preparation_status(&db_path)
            .unwrap()
            .state,
        "idle"
    );
    let connection = crate::db::open(&db_path).unwrap();
    let event: String = connection
            .query_row(
                "SELECT event FROM environment_preparation_events WHERE event LIKE 'tool_candidate_staged:%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(
        event,
        format!(
            "tool_candidate_staged:browser-runtime:{}:{}",
            manifest.payload.digest,
            candidate_cache_key(&manifest.payload).unwrap()
        )
    );
    assert_eq!(
        stage_offline_tool_candidate(
            &db_path,
            &cache,
            &raw,
            &package,
            std::slice::from_ref(&anchor),
            "macos-aarch64"
        )
        .unwrap(),
        destination
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let blob = destination.join("package.blob");
        fs::set_permissions(&blob, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&blob, b"changed").unwrap();
        assert_eq!(
            stage_offline_tool_candidate(
                &db_path,
                &cache,
                &raw,
                &package,
                &[anchor],
                "macos-aarch64"
            )
            .unwrap_err(),
            "tool_package_digest_mismatch"
        );
        assert_eq!(
            crate::db::environment_preparation_status(&db_path)
                .unwrap()
                .state,
            "requires_manual_recovery"
        );
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn same_package_bytes_with_distinct_signed_versions_have_distinct_candidate_paths() {
    let root =
        std::env::temp_dir().join(format!("oviraptor-tool-release-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let db_path = crate::db::initialize(&root).unwrap();
    let cache = root.join("cache");
    let source = root.join("package.blob");
    let (mut manifest, anchor, package, keypair) = fixture();
    fs::write(&source, &package).unwrap();
    let old_raw = sign(&mut manifest, &keypair);
    let first = stage_offline_tool_candidate(
        &db_path,
        &cache,
        &old_raw,
        &package,
        std::slice::from_ref(&anchor),
        "macos-aarch64",
    )
    .unwrap();
    manifest.payload.version = "123.5".into();
    let new_raw = sign(&mut manifest, &keypair);
    let second = stage_offline_tool_candidate_from_file(
        &db_path,
        &cache,
        &new_raw,
        &source,
        std::slice::from_ref(&anchor),
        "macos-aarch64",
    )
    .unwrap();
    assert_ne!(first, second);
    assert_eq!(fs::read(first.join("manifest.json")).unwrap(), old_raw);
    assert_eq!(fs::read(second.join("manifest.json")).unwrap(), new_raw);
    let connection = crate::db::open(&db_path).unwrap();
    let events = connection
            .prepare("SELECT event FROM environment_preparation_events WHERE event LIKE 'tool_candidate_staged:%' ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    assert_eq!(events.len(), 2);
    assert_ne!(events[0], events[1]);
    assert!(events[0].ends_with(first.file_name().unwrap().to_str().unwrap()));
    assert!(events[1].ends_with(second.file_name().unwrap().to_str().unwrap()));
    assert_eq!(
        stage_offline_tool_candidate_from_file(
            &db_path,
            &cache,
            &new_raw,
            &source,
            std::slice::from_ref(&anchor),
            "macos-aarch64",
        )
        .unwrap(),
        second
    );
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&first, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&second, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn candidate_cache_rejects_extra_entries_and_hardlinked_blobs() {
    use std::os::unix::fs::PermissionsExt;

    for corrupt in ["extra_entry", "hardlink"] {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-tool-corrupt-{}-{}",
            corrupt,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let db_path = crate::db::initialize(&root).unwrap();
        let cache = root.join("cache");
        let (mut manifest, anchor, package, keypair) = fixture();
        let raw = sign(&mut manifest, &keypair);
        let destination = stage_offline_tool_candidate(
            &db_path,
            &cache,
            &raw,
            &package,
            std::slice::from_ref(&anchor),
            "macos-aarch64",
        )
        .unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).unwrap();
        if corrupt == "extra_entry" {
            fs::write(destination.join("unexpected"), b"not part of package").unwrap();
        } else {
            let blob = destination.join("package.blob");
            fs::hard_link(&blob, root.join("external-blob")).unwrap();
        }
        assert_eq!(
            stage_offline_tool_candidate(
                &db_path,
                &cache,
                &raw,
                &package,
                &[anchor],
                "macos-aarch64",
            )
            .unwrap_err(),
            "tool_cache_invalid"
        );
        assert_eq!(
            crate::db::environment_preparation_status(&db_path)
                .unwrap()
                .state,
            "requires_manual_recovery"
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn invalid_candidate_does_not_claim_lease_and_live_scan_blocks_staging() {
    let root = std::env::temp_dir().join(format!("oviraptor-tool-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let db_path = crate::db::initialize(&root).unwrap();
    let (mut manifest, anchor, package, keypair) = fixture();
    let raw = sign(&mut manifest, &keypair);
    assert_eq!(
        stage_offline_tool_candidate(
            &db_path,
            &root.join("cache"),
            &raw,
            b"tampered",
            std::slice::from_ref(&anchor),
            "macos-aarch64"
        )
        .unwrap_err(),
        "tool_package_digest_mismatch"
    );
    assert_eq!(
        crate::db::environment_preparation_status(&db_path)
            .unwrap()
            .state,
        "idle"
    );
    let connection = crate::db::open(&db_path).unwrap();
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,status) VALUES('live','queued')",
            [],
        )
        .unwrap();
    assert!(stage_offline_tool_candidate(
        &db_path,
        &root.join("cache"),
        &raw,
        &package,
        &[anchor],
        "macos-aarch64"
    )
    .is_err());
    assert!(!root.join("cache").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn large_offline_candidate_is_streamed_and_stays_inert() {
    let root = std::env::temp_dir().join(format!("oviraptor-tool-large-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let db_path = crate::db::initialize(&root).unwrap();
    let source = root.join("offline-package.blob");
    let file = File::create(&source).unwrap();
    file.set_len((MAX_OPAQUE_PACKAGE_BYTES + 1) as u64).unwrap();
    file.sync_all().unwrap();
    let (mut manifest, anchor, _, keypair) = fixture();
    let mut hash = Sha256::new();
    let zero = [0_u8; 8192];
    for _ in 0..((MAX_OPAQUE_PACKAGE_BYTES + 1) / zero.len()) {
        hash.update(zero);
    }
    hash.update([0]);
    manifest.payload.digest = format!("sha256:{:x}", hash.finalize());
    let raw = sign(&mut manifest, &keypair);
    let destination = stage_offline_tool_candidate_from_file(
        &db_path,
        &root.join("cache"),
        &raw,
        &source,
        &[anchor],
        "macos-aarch64",
    )
    .unwrap();
    assert_eq!(
        fs::metadata(destination.join("package.blob"))
            .unwrap()
            .len(),
        (MAX_OPAQUE_PACKAGE_BYTES + 1) as u64
    );
    assert_eq!(
        crate::db::environment_preparation_status(&db_path)
            .unwrap()
            .state,
        "idle"
    );
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn streamed_candidate_rejects_bad_digest_and_symlink_before_claiming_lease() {
    let root = std::env::temp_dir().join(format!(
        "oviraptor-tool-stream-reject-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir(&root).unwrap();
    let db_path = crate::db::initialize(&root).unwrap();
    let source = root.join("offline.blob");
    let alias = root.join("alias.blob");
    fs::write(&source, b"wrong package bytes").unwrap();
    std::os::unix::fs::symlink(&source, &alias).unwrap();
    let (mut manifest, anchor, package, keypair) = fixture();
    let raw = sign(&mut manifest, &keypair);
    assert_eq!(
        stage_offline_tool_candidate_from_file(
            &db_path,
            &root.join("cache"),
            &raw,
            &source,
            std::slice::from_ref(&anchor),
            "macos-aarch64",
        )
        .unwrap_err(),
        "tool_package_digest_mismatch"
    );
    fs::write(&source, package).unwrap();
    assert_eq!(
        stage_offline_tool_candidate_from_file(
            &db_path,
            &root.join("cache"),
            &raw,
            &alias,
            &[anchor],
            "macos-aarch64",
        )
        .unwrap_err(),
        "tool_package_invalid"
    );
    assert!(!root.join("cache").exists());
    assert_eq!(
        crate::db::environment_preparation_status(&db_path)
            .unwrap()
            .state,
        "idle"
    );
    fs::remove_dir_all(root).unwrap();
}
