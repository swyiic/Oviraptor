// Actual SourceBroker entry over a current sealed analysis view. No role/SDK acceptance claim.
fn dependency_declaration_fixture(
    files: &[(&str, &str)],
) -> (
    PathBuf,
    rusqlite::Connection,
    crate::native_pipeline::tools::SourceBroker,
) {
    let (root, connection, record, plan, draft) = source_ci_publication_fixture();
    let repository = Path::new(&record.source_path);
    for (relative, text) in files {
        let path = repository.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    publish_workbench_fixture(&root, &connection, &record, &plan, &draft, false).unwrap();
    analysis_view_run(&root, &record, |_, engine, _, _, scratch, _| {
        Ok(source_regression_outcome(engine, scratch))
    })
    .unwrap();
    let snapshot = RepositorySnapshot::restore(&connection, &record.scan_id, 1)
        .unwrap()
        .unwrap();
    let view = source_result_view(&connection, &record);
    let broker = crate::native_pipeline::tools::SourceBroker::scoped(
        snapshot,
        view,
        "dependency-parser-only",
        "dependency-parser-only",
        1,
    )
    .unwrap();
    (root, connection, broker)
}

#[test]
fn dependency_source_cargo_reads_only_declared_dependency_tables() {
    let cargo = r#"[package]
name = 'fixture-app'
version = '9.9.9'
edition = '2021'
[package.metadata]
not_a_dependency = 'metadata-value'
[dependencies]
serde = '^1.0'
renamed = { package = 'real-package', version = '>=1, <2', features = ['std'] }
local = { path = 'local-crate' }
inherited = { workspace = true }
[dev-dependencies]
dev = { version = '~2.0' }
[build-dependencies]
cc = '1.2'
[target.'cfg(unix)'.dependencies]
nix = '0.29'
[dependencies.log]
version = '0.4'
optional = true
[workspace.dependencies]
shared = '3'
[profile.release]
opt-level = 3
"#;
    let (root, connection, mut broker) = dependency_declaration_fixture(&[("Cargo.toml", cargo)]);
    let before: i64 = connection
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    let answer = broker
        .call(
            &connection,
            "dependency.get_record",
            &json!({"manifest":"Cargo.toml"}),
        )
        .unwrap();
    assert_eq!(
        answer["dependencies"],
        json!([
            {"name":"cc","requirement":"1.2","scope":"rust"},
            {"name":"dev","requirement":"~2.0","scope":"rust"},
            {"name":"inherited","requirement":"","scope":"rust"},
            {"name":"local","requirement":"","scope":"rust"},
            {"name":"log","requirement":"0.4","scope":"rust"},
            {"name":"nix","requirement":"0.29","scope":"rust"},
            {"name":"renamed","requirement":">=1, <2","scope":"rust"},
            {"name":"serde","requirement":"^1.0","scope":"rust"},
            {"name":"shared","requirement":"3","scope":"rust"}
        ])
    );
    assert_eq!(
        answer["contentHash"].as_str(),
        broker.snapshot.content_hash_of("Cargo.toml")
    );
    assert_eq!(
        answer.as_object().unwrap().len(),
        4,
        "current Native response fields preserved"
    );
    assert!(answer["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r.get("resolvedVersion").is_none()));
    assert_eq!(
        connection
            .query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dependency_source_gradle_reads_groovy_and_kotlin_static_coordinates() {
    let groovy = r#"plugins { id 'java' }
// <artifactId>fake</artifactId><version>999</version>
def documentation = 'ignored:coordinate:0'
dependencies {
    implementation 'org.apache.commons:commons-lang3:3.14.0'
    api group: 'org.example', name: 'api', version: '[1,2)'
    runtimeOnly('org.example:runtime:2.0') { exclude group: 'ignored', module: 'excluded' }
}
"#;
    let kotlin = r#"plugins { id("java") }
/* implementation("fake:comment:999") */
dependencies {
    implementation("org.example:core:1.+")
    testImplementation(platform("org.junit:junit-bom:5.10.0"))
    runtimeOnly(group = "org.example", name = "runtime", version = "2.0")
}
"#;
    let (root, connection, mut broker) =
        dependency_declaration_fixture(&[("build.gradle", groovy), ("build.gradle.kts", kotlin)]);
    for (manifest, expected) in [
        (
            "build.gradle",
            json!([
                {"name":"org.apache.commons:commons-lang3","requirement":"3.14.0","scope":"java"},
                {"name":"org.example:api","requirement":"[1,2)","scope":"java"},
                {"name":"org.example:runtime","requirement":"2.0","scope":"java"}
            ]),
        ),
        (
            "build.gradle.kts",
            json!([
                {"name":"org.example:core","requirement":"1.+","scope":"java"},
                {"name":"org.junit:junit-bom","requirement":"5.10.0","scope":"java"},
                {"name":"org.example:runtime","requirement":"2.0","scope":"java"}
            ]),
        ),
    ] {
        let answer = broker
            .call(
                &connection,
                "dependency.get_record",
                &json!({"manifest":manifest}),
            )
            .unwrap();
        assert_eq!(answer["dependencies"], expected, "{answer}");
        assert_eq!(
            answer["contentHash"].as_str(),
            broker.snapshot.content_hash_of(manifest)
        );
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dependency_source_unresolved_or_invalid_declarations_are_not_empty_success() {
    let cases = [
        ("Cargo.toml", "[dependencies]\nserde = true\n"),
        ("Cargo.toml", "[dependencies\nserde = '1'\n"),
        (
            "build.gradle.kts",
            "dependencies { implementation(libs.example) }",
        ),
        (
            "build.gradle",
            "dependencies { implementation 'org.example:api:$version' }",
        ),
        (
            "build.gradle",
            "dependencies { customConfiguration 'org.example:api:1' }",
        ),
        (
            "build.gradle",
            "dependencies { implementation 'org.example:api:1' + suffix }",
        ),
        (
            "build.gradle.kts",
            "dependencies { runtimeOnly(group = \"org.example\", name = \"api\", version = \"1\", extra = \"unresolved\") }",
        ),
    ];
    for (manifest, text) in cases {
        let (root, connection, mut broker) = dependency_declaration_fixture(&[(manifest, text)]);
        let denial = broker
            .call(
                &connection,
                "dependency.get_record",
                &json!({"manifest":manifest}),
            )
            .unwrap_err();
        assert_eq!(denial.code, "dependency_declaration_invalid");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn dependency_source_answers_stay_pinned_and_do_not_run_manifest_scripts() {
    let cargo = "[package]\nname='fixture'\nversion='9.9'\n[dependencies]\nserde='^1.0'\n";
    let gradle = "tasks.register('writeMarker') { doLast { file('dependency-script-ran').text = 'unexpected' } }\ndependencies { implementation 'org.example:api:1.+' }\n";
    let (root, connection, mut broker) =
        dependency_declaration_fixture(&[("Cargo.toml", cargo), ("build.gradle", gradle)]);
    let first = broker
        .call(
            &connection,
            "dependency.get_record",
            &json!({"manifest":"Cargo.toml"}),
        )
        .unwrap();
    fs::write(
        broker.snapshot.root.join("Cargo.toml"),
        "[dependencies]\nforeign='99'\n",
    )
    .unwrap();
    assert_eq!(
        broker
            .call(
                &connection,
                "dependency.get_record",
                &json!({"manifest":"Cargo.toml"})
            )
            .unwrap(),
        first
    );
    let answer = broker
        .call(
            &connection,
            "dependency.get_record",
            &json!({"manifest":"build.gradle"}),
        )
        .unwrap();
    assert_eq!(answer["dependencies"][0]["requirement"], "1.+");
    assert!(!broker.snapshot.root.join("dependency-script-ran").exists());
    assert!(broker
        .call(
            &connection,
            "dependency.get_record",
            &json!({"manifest":"../Cargo.toml"})
        )
        .is_err());
    assert!(broker
        .call(
            &connection,
            "dependency.get_record",
            &json!({"manifest":"not-selected.gradle"})
        )
        .is_err());
    assert!(broker
        .call(&connection, "shell.exec", &json!({"command":"gradle"}))
        .is_err());
    // Replace only this private fixture's sealed file, never a real repository/CAS.
    let frozen = broker.snapshot.frozen_root.join("Cargo.toml");
    fs::remove_file(&frozen).unwrap();
    fs::write(&frozen, "[dependencies]\nforeign='99'\n").unwrap();
    let denial = broker
        .call(
            &connection,
            "dependency.get_record",
            &json!({"manifest":"Cargo.toml"}),
        )
        .unwrap_err();
    assert_eq!(denial.code, "snapshot_integrity");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
