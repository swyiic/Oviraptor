//! Exercise real process outcomes instead of pre-inserting a successful database row.
use super::*;
use crate::native_pipeline::analyzer::{self, AnalyzerEngine, AnalyzerSpec, AnalyzerStatus};

fn analyzer_spec(root: &Path, engine: AnalyzerEngine) -> AnalyzerSpec {
    let repository = fixture_repository(root);
    let rule_pack = root.join("rules");
    write_file(&rule_pack, "fixture.yaml", "rules: []\n");
    AnalyzerSpec {
        engine,
        program: root.join("fake-analyzer"),
        image: None,
        rule_pack,
        languages: vec!["python".into()],
        repository,
        scratch_dir: scratch_dir(root),
        scan_id: "analyzer-regression".into(),
        attempt_number: 1,
        limits: process_limits(5, 4096),
    }
}

#[cfg(unix)]
fn install_fake(spec: &AnalyzerSpec, exit: i32) {
    use std::os::unix::fs::PermissionsExt;
    // This fixture only writes in the isolated test scratch directory. It emulates
    // both a host analyzer and a container launcher without contacting a daemon.
    if spec.image.is_some() {
        install_fake_container(spec, if exit == 0 { "ok" } else { "start_failed" });
        return;
    }
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$*" >> launches.log
if [ "$1" = '--version' ] || [ "$1" = 'version' ]; then
  printf 'host-launcher-version\n'
  exit 0
fi
last=''
output=''
for arg in "$@"; do
  if [ "$last" = '--output' ]; then output="$arg"; fi
  last="$arg"
done
if [ "$last" = '--version' ] || [ "$last" = 'version' ]; then
  printf 'image-analyzer-version\n'
  exit 0
fi
if [ '{exit}' != '0' ]; then exit {exit}; fi
case "$output" in /out/*) output="./${{output#/out/}}";; esac
if [ -n "$output" ]; then
  printf '{{"version":"2.1.0","runs":[{{"tool":{{"driver":{{"name":"fixture"}}}},"results":[]}}]}}' > "$output"
fi
exit 0
"#
    );
    fs::write(&spec.program, script).unwrap();
    fs::set_permissions(&spec.program, fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
fn install_fake_container(spec: &AnalyzerSpec, mode: &str) {
    use std::os::unix::fs::PermissionsExt;
    let script = r#"#!/bin/sh
printf '%s\n' "$*" >> launches.log
id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
mode='MODE'
case "$1" in
  create)
    prev=''
    for arg in "$@"; do
      [ "$prev" = '--name' ] && printf '%s' "$arg" > container.name
      [ "$prev" = '--label' ] && printf '%s' "${arg#*=}" > container.owner
      prev="$arg"
    done
    printf '%s\n' "$@" > create.args
    touch container.exists
    [ "$mode" = 'create_timeout' ] && sleep 2
    printf '%s\n' "$id"
    ;;
  start)
    touch container.started
    [ "$mode" = 'start_failed' ] && exit 2
    [ "$mode" = 'start_timeout' ] && sleep 2
    prev=''
    output=''
    while IFS= read -r arg; do
      [ "$prev" = '--output' ] && output="$arg"
      prev="$arg"
    done < create.args
    if [ "$prev" = '--version' ] || [ "$prev" = 'version' ]; then
      printf 'image-analyzer-version\n'
    elif [ -n "$output" ]; then
      output="./${output#/out/}"
      printf '{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"fixture"}},"results":[]}]}' > "$output"
    fi
    ;;
  container)
    case "$2" in
      inspect)
        [ -f container.exists ] || exit 1
        owner=$(cat container.owner)
        [ "$mode" = 'foreign_owner' ] && owner=someone-else
        printf '[{"Id":"%s","Name":"/%s","Config":{"Labels":{"io.oviraptor.native-owner":"%s"}}}]\n' "$id" "$(cat container.name)" "$owner"
        ;;
      rm)
        [ "$mode" = 'cleanup_failed' ] && exit 2
        rm -f container.exists
        ;;
      ls)
        [ "$mode" = 'daemon_lost' ] && exit 2
        [ -f container.exists ] && printf '%s\n' "$id"
        ;;
    esac
    ;;
esac
exit 0
"#.replace("MODE", mode);
    fs::write(&spec.program, script).unwrap();
    fs::set_permissions(&spec.program, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
#[cfg(unix)]
fn analyzer_container_steps_leave_confirmed_cleanup_receipts() {
    let root = sandbox("container-receipts");
    let connection = open_connection(&initialize_db(&root));
    let mut spec = analyzer_spec(&root, AnalyzerEngine::Semgrep);
    spec.image = Some(format!("fixture/analyzer@sha256:{}", "a".repeat(64)));
    install_fake_container(&spec, "ok");
    let outcome = analyzer::run(&connection, &spec, &|| false).unwrap();
    assert!(outcome.produced_results());
    let receipts: i64 = connection.query_row(
        "SELECT count(*) FROM analyzer_container_receipts WHERE cleanup_status='confirmed' AND scan_id=?1",
        [&spec.scan_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        receipts, 2,
        "version probe and analysis both require their own cleanup proof"
    );
    assert!(!spec.scratch_dir.join("container.exists").exists());
    let log = fs::read_to_string(spec.scratch_dir.join("launches.log")).unwrap();
    assert!(!log.lines().any(|line| line.starts_with("run ")));
    assert!(log.contains("container rm --force"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn analyzer_cleanup_failure_or_wrong_owner_blocks_results_and_retry() {
    for mode in ["cleanup_failed", "foreign_owner", "daemon_lost"] {
        let root = sandbox(mode);
        let connection = open_connection(&initialize_db(&root));
        let mut spec = analyzer_spec(&root, AnalyzerEngine::Semgrep);
        spec.image = Some(format!("fixture/analyzer@sha256:{}", "a".repeat(64)));
        install_fake_container(&spec, mode);
        let first = analyzer::run(&connection, &spec, &|| false);
        assert!(first.is_err() || !first.unwrap().produced_results());
        let log_before = fs::read(spec.scratch_dir.join("launches.log")).unwrap();
        let second = analyzer::run(&connection, &spec, &|| false);
        assert!(
            second.is_err(),
            "unresolved cleanup must block new container creation"
        );
        assert_eq!(
            fs::read(spec.scratch_dir.join("launches.log")).unwrap(),
            log_before
        );
        let pending: i64 = connection.query_row(
            "SELECT count(*) FROM analyzer_container_receipts WHERE cleanup_status<>'confirmed'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(pending, 1);
        if mode == "foreign_owner" {
            assert!(!spec.scratch_dir.join("container.started").exists());
            assert!(!String::from_utf8_lossy(&log_before).contains("container rm"));
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
#[cfg(unix)]
fn interrupted_container_create_never_starts_and_records_uncertainty() {
    let root = sandbox("create-timeout");
    let connection = open_connection(&initialize_db(&root));
    let mut spec = analyzer_spec(&root, AnalyzerEngine::Semgrep);
    spec.image = Some(format!("fixture/analyzer@sha256:{}", "a".repeat(64)));
    spec.limits.timeout = Duration::from_millis(100);
    install_fake_container(&spec, "create_timeout");
    let first = analyzer::run(&connection, &spec, &|| false);
    assert!(first.is_err() || !first.unwrap().produced_results());
    assert!(!spec.scratch_dir.join("container.started").exists());
    let status: String = connection
        .query_row(
            "SELECT cleanup_status FROM analyzer_container_receipts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(
        status, "confirmed",
        "an interrupted daemon create request has an uncertain outcome"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn analyzer_success_is_recorded_and_the_real_second_run_is_reused() {
    for engine in [AnalyzerEngine::Semgrep, AnalyzerEngine::CodeQL] {
        let root = sandbox("analyzer-success");
        let connection = open_connection(&initialize_db(&root));
        let spec = analyzer_spec(&root, engine);
        install_fake(&spec, 0);
        let first = analyzer::run(&connection, &spec, &|| false).unwrap();
        assert_eq!(first.status, AnalyzerStatus::Ran { exit: Some(0) });
        assert!(first.produced_results());
        assert!(!first.reused);
        let log = fs::read(spec.scratch_dir.join("launches.log")).unwrap();
        let second = analyzer::run(&connection, &spec, &|| false).unwrap();
        assert!(second.reused && second.produced_results());
        assert_eq!(
            fs::read(spec.scratch_dir.join("launches.log")).unwrap(),
            log
        );
        assert_eq!(table_count(&connection, "analyzer_runs"), 1);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
#[cfg(unix)]
fn analyzer_success_after_failure_updates_the_persisted_outcome() {
    let root = sandbox("analyzer-retry");
    let connection = open_connection(&initialize_db(&root));
    let spec = analyzer_spec(&root, AnalyzerEngine::Semgrep);
    install_fake(&spec, 2);
    assert!(!analyzer::run(&connection, &spec, &|| false)
        .unwrap()
        .produced_results());
    install_fake(&spec, 0);
    assert!(analyzer::run(&connection, &spec, &|| false)
        .unwrap()
        .produced_results());
    let third = analyzer::run(&connection, &spec, &|| false).unwrap();
    assert!(
        third.reused,
        "a failed row must not mask a later successful retry"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn pinned_analyzer_uses_container_paths_and_probes_the_image_version() {
    for engine in [AnalyzerEngine::Semgrep, AnalyzerEngine::CodeQL] {
        let root = sandbox("analyzer-container");
        let connection = open_connection(&initialize_db(&root));
        let mut spec = analyzer_spec(&root, engine);
        spec.image = Some(format!("fixture/analyzer@sha256:{}", "a".repeat(64)));
        install_fake(&spec, 0);
        let steps = analyzer::planned_args(&spec).unwrap();
        for step in &steps {
            assert!(
                step.iter().any(|arg| arg == "--pull=never"),
                "runtime must not download an image implicitly"
            );
            let image_index = step
                .iter()
                .position(|arg| Some(arg) == spec.image.as_ref())
                .unwrap();
            assert!(
                step[image_index + 1..]
                    .iter()
                    .all(|arg| !arg.contains(spec.repository.to_str().unwrap())
                        && !arg.contains(spec.scratch_dir.to_str().unwrap())),
                "host paths cannot be resolved inside a container: {step:?}"
            );
            if let Some(index) = step.iter().position(|arg| arg == "--output") {
                assert_eq!(step[index + 1], format!("/out/{}.sarif", engine.as_str()));
            }
        }
        let outcome = analyzer::run(&connection, &spec, &|| false).unwrap();
        assert_eq!(outcome.version, "image-analyzer-version");
        assert!(outcome.produced_results());
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn pinned_analyzer_rejects_missing_or_short_sha256_digests() {
    let root = sandbox("analyzer-digest");
    let mut spec = analyzer_spec(&root, AnalyzerEngine::Semgrep);
    for image in [
        format!("fixture/analyzer@{}", "a".repeat(64)),
        format!("fixture/analyzer@sha256:{}", "a".repeat(32)),
        format!("fixture/analyzer@sha256:{}", "a".repeat(65)),
    ] {
        spec.image = Some(image);
        assert!(analyzer::planned_args(&spec).is_err());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_rejects_scratch_within_the_source_without_creating_it() {
    let root = sandbox("snapshot-scratch");
    let repository = fixture_repository(&root);
    let nested = repository.join("analysis-output");
    let outcome = RepositorySnapshot::capture(&repository, &nested, None);
    assert!(
        outcome.is_err(),
        "scratch must never be created in the user's source tree"
    );
    assert!(!nested.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn modified_cached_sarif_is_rejected_without_launching_again() {
    let root = sandbox("sarif-cache-integrity");
    let connection = open_connection(&initialize_db(&root));
    let spec = analyzer_spec(&root, AnalyzerEngine::Semgrep);
    install_fake(&spec, 0);
    let first = analyzer::run(&connection, &spec, &|| false).unwrap();
    assert!(first.produced_results());
    let before = fs::read(spec.scratch_dir.join("launches.log")).unwrap();
    fs::write(first.sarif_path.unwrap(), "{\"tampered\":true}").unwrap();
    let reused = analyzer::run(&connection, &spec, &|| false);
    assert!(
        reused.is_err(),
        "file existence alone does not prove cached evidence identity"
    );
    assert_eq!(
        fs::read(spec.scratch_dir.join("launches.log")).unwrap(),
        before
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn successful_process_cannot_relabel_a_previous_sarif_as_its_new_output() {
    let root = sandbox("sarif-stale-output");
    let connection = open_connection(&initialize_db(&root));
    let spec = analyzer_spec(&root, AnalyzerEngine::Semgrep);
    fs::write(analyzer::sarif_path(&spec), "previous-output").unwrap();
    install_fake(&spec, 0);
    // This program succeeds but never writes SARIF.
    fs::write(
        &spec.program,
        "#!/bin/sh\nprintf 'fixture version\\n'\nexit 0\n",
    )
    .unwrap();
    let result = analyzer::run(&connection, &spec, &|| false).unwrap();
    assert!(
        !result.produced_results(),
        "an old file is not evidence that this run emitted results"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn concurrent_connections_cannot_launch_the_same_analyzer_output_twice() {
    let root = sandbox("analyzer-live-claim");
    let db_path = initialize_db(&root);
    let spec = analyzer_spec(&root, AnalyzerEngine::Semgrep);
    install_fake(&spec, 0);
    let script = fs::read_to_string(&spec.program)
        .unwrap()
        .replace("#!/bin/sh\n", "#!/bin/sh\nsleep 0.3\n");
    fs::write(&spec.program, script).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let barrier = barrier.clone();
            let spec = spec.clone();
            let db_path = db_path.clone();
            std::thread::spawn(move || {
                let connection = open_connection(&db_path);
                barrier.wait();
                analyzer::run(&connection, &spec, &|| false)
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        1,
        "a live owner must exclude a second execution: {results:?}"
    );
    assert!(results.iter().any(|result| result
        .as_ref()
        .err()
        .is_some_and(|error| error.contains("analyzer_invocation_in_progress"))));
    assert_eq!(
        fs::read_to_string(spec.scratch_dir.join("launches.log"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    // Closing the winner's guard releases the lock; later calls may validate/reuse.
    assert!(
        analyzer::run(&open_connection(&db_path), &spec, &|| false)
            .unwrap()
            .reused
    );
    fs::remove_dir_all(root).unwrap();
}
