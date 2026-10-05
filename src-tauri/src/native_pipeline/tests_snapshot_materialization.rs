use super::*;

#[test]
fn frozen_git_diff_survives_repository_and_branch_changes() {
    let root = sandbox("frozen-git-diff");
    let repo = fixture_repository(&root);
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .current_dir(&repo)
            .args([
                "-c",
                "user.name=Snapshot Fixture",
                "-c",
                "user.email=fixture@invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    git(&["init"]);
    git(&["add", "."]);
    git(&["commit", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    write_file(&repo, "src/server.js", "const changed = true;\n");
    git(&["add", "."]);
    git(&["commit", "-m", "head"]);
    let snapshot = capture_at(&repo, &root.join("scratch"), Some(&base));
    assert_eq!(snapshot.changed_files, Some(vec!["src/server.js".into()]));
    let connection = open_connection(&initialize_db(&root));
    snapshot.store(&connection, "frozen-diff", 1).unwrap();
    write_file(&repo, "new.py", "print('later commit')\n");
    git(&["add", "."]);
    git(&["commit", "-m", "later"]);
    let restored = RepositorySnapshot::restore(&connection, "frozen-diff", 1)
        .unwrap()
        .unwrap();
    assert_eq!(restored.changed_files, snapshot.changed_files);
    assert_eq!(restored.commit_sha, snapshot.commit_sha);
    restored.verify_frozen().unwrap();
    assert!(restored.verify_unchanged().is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn frozen_source_is_an_independent_read_only_copy_and_restores_with_gaps() {
    let root = sandbox("materialized");
    let repo = fixture_repository(&root);
    let snapshot = capture_at(&repo, &root.join("scratch"), None);
    let path = snapshot.frozen_root.join("src/server.js");
    let original = fs::read(&path).unwrap();
    assert_ne!(snapshot.root, snapshot.frozen_root);
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    assert!(!snapshot.frozen_root.join("node_modules").exists());
    write_file(&repo, "src/server.js", "changed source");
    assert_eq!(fs::read(&path).unwrap(), original);
    snapshot.verify_frozen().unwrap();
    assert!(snapshot.verify_unchanged().is_err());
    let db_path = initialize_db(&root);
    let connection = open_connection(&db_path);
    snapshot.store(&connection, "materialized", 1).unwrap();
    let restored = RepositorySnapshot::restore(&connection, "materialized", 1)
        .unwrap()
        .unwrap();
    assert_eq!(restored.frozen_root, snapshot.frozen_root);
    assert_eq!(restored.gaps, snapshot.gaps);
    restored.verify_frozen().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unexpected_file_or_replaced_frozen_file_fails_integrity() {
    let root = sandbox("frozen-tampering");
    let repo = fixture_repository(&root);
    let snapshot = capture_at(&repo, &root.join("scratch"), None);
    write_file(&snapshot.frozen_root, "injected.py", "unexpected");
    assert!(snapshot.verify_frozen().is_err());
    fs::remove_file(snapshot.frozen_root.join("injected.py")).unwrap();
    fs::remove_file(snapshot.frozen_root.join("src/server.js")).unwrap();
    write_file(&snapshot.frozen_root, "src/server.js", "replacement");
    assert!(snapshot.verify_frozen().is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn historical_manifest_without_materialized_source_is_not_executable() {
    let root = sandbox("historical-snapshot");
    let repo = fixture_repository(&root);
    let snapshot = capture_at(&repo, &root.join("scratch"), None);
    let connection = open_connection(&initialize_db(&root));
    snapshot.store(&connection, "historical", 1).unwrap();
    connection
        .execute("UPDATE source_snapshots SET frozen_root=''", [])
        .unwrap();
    let restored = RepositorySnapshot::restore(&connection, "historical", 1)
        .unwrap()
        .unwrap();
    assert!(restored
        .verify_frozen()
        .unwrap_err()
        .contains("materialization_missing"));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn frozen_file_replaced_with_an_identical_external_symlink_is_rejected() {
    let root = sandbox("frozen-symlink");
    let repo = fixture_repository(&root);
    let snapshot = capture_at(&repo, &root.join("scratch"), None);
    let path = snapshot.frozen_root.join("src/server.js");
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(repo.join("src/server.js"), &path).unwrap();
    assert!(snapshot.verify_frozen().is_err());
    fs::remove_dir_all(root).unwrap();
}
