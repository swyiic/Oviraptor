use super::*;

fn git(repo: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(repo)
        .args([
            "-c",
            "user.name=Scope Fixture",
            "-c",
            "user.email=scope@invalid",
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
}

fn repository(tag: &str) -> (PathBuf, PathBuf, String) {
    let root = sandbox(tag);
    let repo = root.join("repo");
    for name in [
        "committed.py",
        "dirty.py",
        "staged.py",
        "deleted.py",
        "rename-old.py",
        "untouched.py",
    ] {
        write_file(&repo, name, &format!("print('{name}')\n"));
    }
    git(&repo, &["init"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    (root, repo, base)
}

#[test]
fn snapshot_diff_includes_committed_staged_dirty_untracked_deleted_and_both_rename_paths() {
    let (root, repo, base) = repository("diff-worktree");
    write_file(&repo, "committed.py", "print('committed change')\n");
    git(&repo, &["add", "committed.py"]);
    git(&repo, &["commit", "-m", "head"]);
    write_file(&repo, "staged.py", "print('staged change')\n");
    git(&repo, &["add", "staged.py"]);
    write_file(&repo, "dirty.py", "print('dirty change')\n");
    write_file(&repo, "new file.py", "print('untracked')\n");
    fs::remove_file(repo.join("deleted.py")).unwrap();
    git(&repo, &["mv", "rename-old.py", "rename-new.py"]);
    let snapshot = capture_at(&repo, &root.join("scratch"), Some(&base));
    let expected: Vec<String> = [
        "committed.py",
        "deleted.py",
        "dirty.py",
        "new file.py",
        "rename-new.py",
        "rename-old.py",
        "staged.py",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(snapshot.changed_files, Some(expected));
    assert!(
        snapshot.files.iter().any(|f| f.path == "untouched.py"),
        "provenance stays whole-repository"
    );
    assert!(!snapshot.files.iter().any(|f| f.path == "deleted.py"));
    snapshot.verify_unchanged().unwrap();
    let connection = open_connection(&initialize_db(&root));
    snapshot.store(&connection, "diff-worktree", 1).unwrap();
    let restored = RepositorySnapshot::restore(&connection, "diff-worktree", 1)
        .unwrap()
        .unwrap();
    assert_eq!(restored.changed_files, snapshot.changed_files);
    restored.verify_unchanged().unwrap();
    write_file(&repo, "untouched.py", "print('changed after freeze')\n");
    assert!(
        restored.verify_unchanged().is_err(),
        "scope must not weaken original source verification"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_diff_preserves_nul_delimited_names_and_repository_relative_subdirectory_paths() {
    let (root, repo, base) = repository("diff-subdirectory");
    write_file(&repo, "component/normal.py", "base\n");
    write_file(&repo, "component/\nleading.py", "base\n");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "component"]);
    let component_base = git(&repo, &["rev-parse", "HEAD"]);
    write_file(&repo, "component/\nleading.py", "changed\n");
    write_file(&repo, "component/new\nfile.py", "new\n");
    write_file(&repo, "dirty.py", "outside selected source root\n");
    let snapshot = capture_at(
        &repo.join("component"),
        &root.join("scratch"),
        Some(&component_base),
    );
    assert_eq!(
        snapshot.changed_files,
        Some(vec!["\nleading.py".into(), "new\nfile.py".into()])
    );
    assert_eq!(snapshot.base_sha, component_base);
    assert_ne!(snapshot.base_sha, base);
    snapshot.verify_unchanged().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_diff_distinguishes_empty_invalid_and_hidden_index_coverage() {
    let (root, repo, base) = repository("diff-coverage");
    let clean = capture_at(&repo, &root.join("clean"), Some(&base));
    assert_eq!(clean.changed_files, Some(vec![]));
    for invalid in ["unknown-base", "--help", ""] {
        let snapshot = capture_at(&repo, &root.join(Uuid::new_v4().to_string()), Some(invalid));
        assert_eq!(snapshot.changed_files, None);
        assert!(!snapshot.diff_base.is_usable());
    }
    for flag in ["--assume-unchanged", "--skip-worktree"] {
        git(&repo, &["update-index", flag, "dirty.py"]);
        write_file(&repo, "dirty.py", "print('hidden working tree edit')\n");
        let snapshot = capture_at(&repo, &root.join(Uuid::new_v4().to_string()), Some(&base));
        assert_eq!(
            snapshot.changed_files, None,
            "Git flags must not turn hidden edits into no changes"
        );
        assert!(snapshot.gaps.iter().any(|g| g == "diff_index_hidden_paths"));
        let flags = git(&repo, &["ls-files", "-v", "dirty.py"]);
        assert!(
            flags.starts_with('h') || flags.starts_with('S'),
            "capture must not rewrite the operator's index"
        );
        // update-index applies only the last flag operation when combined.
        git(
            &repo,
            &["update-index", "--no-assume-unchanged", "dirty.py"],
        );
        git(&repo, &["update-index", "--no-skip-worktree", "dirty.py"]);
    }
    let visible = capture_at(&repo, &root.join("visible"), Some(&base));
    assert_eq!(visible.changed_files, Some(vec!["dirty.py".into()]));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_diff_bad_index_is_a_gap_and_ignored_untracked_files_are_not_selected() {
    let (root, repo, _) = repository("diff-bad-index");
    write_file(&repo, ".gitignore", "ignored.py\n");
    git(&repo, &["add", ".gitignore"]);
    git(&repo, &["commit", "-m", "ignore"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    write_file(&repo, "ignored.py", "not selected by Git\n");
    write_file(&repo, "included.py", "new untracked source\n");
    let snapshot = capture_at(&repo, &root.join("good"), Some(&base));
    assert_eq!(snapshot.changed_files, Some(vec!["included.py".into()]));
    let index = repo.join(".git/index");
    let before = fs::read(&index).unwrap();
    fs::write(&index, b"broken fixture index").unwrap();
    let snapshot = capture_at(&repo, &root.join("broken"), Some(&base));
    assert_eq!(snapshot.changed_files, None);
    assert!(snapshot.gaps.iter().any(|g| g == "diff_index_unavailable"));
    assert_eq!(fs::read(&index).unwrap(), b"broken fixture index");
    fs::write(&index, before).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_diff_nonancestor_base_is_not_an_empty_success() {
    let (root, repo, base) = repository("diff-nonancestor");
    git(&repo, &["checkout", "-b", "other"]);
    write_file(&repo, "other.py", "other branch\n");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "other"]);
    let nonancestor = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["checkout", "--detach", &base]);
    let snapshot = capture_at(&repo, &root.join("scratch"), Some(&nonancestor));
    assert_eq!(
        snapshot.diff_base,
        crate::native_pipeline::snapshot::DiffBaseState::NotAncestor
    );
    assert_eq!(snapshot.changed_files, None);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn snapshot_diff_never_invokes_external_diff_textconv_or_fsmonitor() {
    let (root, repo, base) = repository("diff-no-hooks");
    let marker = root.join("hook-executed");
    let command = format!("touch '{}'", marker.display());
    git(&repo, &["config", "diff.external", &command]);
    git(&repo, &["config", "diff.scopefixture.textconv", &command]);
    git(&repo, &["config", "core.fsmonitor", &command]);
    write_file(&repo, ".gitattributes", "*.py diff=scopefixture\n");
    write_file(&repo, "dirty.py", "print('dirty')\n");
    let snapshot = capture_at(&repo, &root.join("scratch"), Some(&base));
    assert_eq!(
        snapshot.changed_files,
        Some(vec![".gitattributes".into(), "dirty.py".into()])
    );
    assert!(
        !marker.exists(),
        "repository configuration cannot execute during scope discovery"
    );
    // Positive control: this repository really has an executable monitor. The
    // fixture is not passing merely because Git ignored the configuration.
    git(&repo, &["status", "--porcelain"]);
    assert!(
        marker.exists(),
        "unrestricted Git must exercise the fixture hook"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_diff_broker_uses_only_persisted_working_tree_paths_and_preserves_unknown_coverage() {
    let (root, repo, base) = repository("diff-broker");
    write_file(&repo, "dirty.py", "changed before capture\n");
    write_file(&repo, "untracked.py", "new before capture\n");
    let snapshot = capture_at(&repo, &root.join("scratch"), Some(&base));
    let connection = open_connection(&initialize_db(&root));
    snapshot.store(&connection, "diff-broker", 1).unwrap();
    let restored = RepositorySnapshot::restore(&connection, "diff-broker", 1)
        .unwrap()
        .unwrap();
    write_file(&repo, "later.py", "not part of frozen list\n");
    let mut broker = crate::native_pipeline::tools::SourceBroker::new(
        restored,
        "diff-broker",
        1,
        "root",
        "reader",
        1,
    );
    let response = broker
        .call(&connection, "git.changed_files", &json!({}))
        .unwrap();
    assert_eq!(response["files"], json!(["dirty.py", "untracked.py"]));
    assert_eq!(response["fileCount"], 2);
    assert_eq!(response["truncated"], false);
    assert!(broker.snapshot.verify_unchanged().is_err());
    git(&repo, &["update-index", "--assume-unchanged", "dirty.py"]);
    let hidden = capture_at(&repo, &root.join("hidden"), Some(&base));
    hidden.store(&connection, "diff-broker", 2).unwrap();
    let restored = RepositorySnapshot::restore(&connection, "diff-broker", 2)
        .unwrap()
        .unwrap();
    let mut broker = crate::native_pipeline::tools::SourceBroker::new(
        restored,
        "diff-broker",
        2,
        "root",
        "reader",
        1,
    );
    assert_eq!(
        broker
            .call(&connection, "git.changed_files", &json!({}))
            .unwrap_err()
            .code,
        "diff_base_unavailable"
    );
    assert!(broker
        .known_gaps()
        .iter()
        .any(|gap| gap == "diff_index_hidden_paths"));
    fs::remove_dir_all(root).unwrap();
}
