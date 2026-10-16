use super::*;

struct Fixture {
    root: PathBuf,
    path: PathBuf,
    db: Connection,
    registry: Registry,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("oviraptor-custody-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let root = fs::canonicalize(root).unwrap();
        let path = root.join("private.sqlite3");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE own_store(instance TEXT NOT NULL);INSERT INTO own_store VALUES('original');CREATE TABLE effects(value TEXT);").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let registry = Registry::default();
        Self {
            root,
            path,
            db,
            registry,
        }
    }

    fn register(&self) {
        self.registry
            .register(
                &self.path,
                &self.db,
                "original".into(),
                Arc::new(|db| {
                    let instances: Vec<String> = db
                        .prepare("SELECT instance FROM main.own_store")
                        .map_err(|e| e.to_string())?
                        .query_map([], |r| r.get(0))
                        .map_err(|e| e.to_string())?
                        .collect::<Result<_, _>>()
                        .map_err(|e| e.to_string())?;
                    if instances == ["original"] {
                        Ok(())
                    } else {
                        Err("original_owner_changed".into())
                    }
                }),
            )
            .unwrap();
    }

    fn open(&self) -> rusqlite::Result<Connection> {
        self.registry
            .open(
                &self.path,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            )
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
#[test]
fn registered_store_preserves_private_writer_flags_authorizer_and_wal_notifications() {
    let f = Fixture::new();
    f.register();
    let db = f.open().unwrap();
    crate::collaboration_events::install_commit_notifications(&db);
    db.execute("INSERT INTO effects VALUES('permitted')", [])
        .unwrap();
    db.authorizer(Some(
        |context: rusqlite::hooks::AuthContext<'_>| match context.action {
            rusqlite::hooks::AuthAction::Insert { .. } => rusqlite::hooks::Authorization::Deny,
            _ => rusqlite::hooks::Authorization::Allow,
        },
    ))
    .unwrap();
    assert!(db
        .execute("INSERT INTO effects VALUES('denied')", [])
        .is_err());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM effects", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(f
        .registry
        .open(
            &f.root.join("unregistered.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_WRITE
        )
        .is_none());
}

#[cfg(unix)]
#[test]
fn missing_registered_store_is_never_created_even_when_original_flags_request_create() {
    let f = Fixture::new();
    f.register();
    let moved = f.root.join("moved.sqlite3");
    fs::rename(&f.path, &moved).unwrap();
    let before = fs::read(&moved).unwrap();
    assert!(f.open().is_err());
    assert!(!f.path.exists());
    assert_eq!(fs::read(moved).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn foreign_live_replacement_and_copied_identity_cannot_receive_a_low_level_write() {
    let f = Fixture::new();
    f.register();
    let moved = f.root.join("moved.sqlite3");
    fs::rename(&f.path, &moved).unwrap();
    fs::copy(&moved, &f.path).unwrap();
    let foreign = fs::read(&f.path).unwrap();
    let original = fs::read(&moved).unwrap();
    for flags in [
        OpenFlags::SQLITE_OPEN_READ_WRITE,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    ] {
        assert!(f.registry.open(&f.path, flags).unwrap().is_err());
    }
    assert_eq!(fs::read(&f.path).unwrap(), foreign);
    assert_eq!(fs::read(&moved).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn commit_guard_survives_service_drop_and_refuses_an_opened_file_that_later_moves() {
    let f = Fixture::new();
    f.register();
    let db = f.open().unwrap();
    crate::collaboration_events::install_commit_notifications(&db);
    let moved = f.root.join("moved.sqlite3");
    fs::rename(&f.path, &moved).unwrap();
    fs::copy(&moved, &f.path).unwrap();
    let foreign = fs::read(&f.path).unwrap();
    let original = fs::read(&moved).unwrap();
    // The connection retains its own custody Arc; neither the caller nor an
    // independent WAL hook can remove the commit fence.
    drop(f.registry.0.lock().unwrap().take());
    assert!(db
        .execute_batch("BEGIN IMMEDIATE;INSERT INTO effects VALUES('never committed');COMMIT;")
        .is_err());
    assert!(f
        .db
        .execute_batch(
            "BEGIN IMMEDIATE;INSERT INTO effects VALUES('retained never committed');COMMIT;"
        )
        .is_err());
    drop(db);
    assert_eq!(fs::read(&f.path).unwrap(), foreign);
    assert_eq!(fs::read(&moved).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn registered_alias_symlink_and_hardlink_are_rejected_without_touching_the_store() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.register();
    let before = fs::read(&f.path).unwrap();
    let alias = f.root.join("alias.sqlite3");
    symlink(&f.path, &alias).unwrap();
    assert!(f
        .registry
        .open(&alias, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .unwrap()
        .is_err());
    let dotted = f.root.join(".").join("private.sqlite3");
    // Path equality normalizes '.', so an existing canonical path is retained;
    // an explicit symlink remains a distinct and rejected path.
    if dotted != f.path {
        assert!(f
            .registry
            .open(&dotted, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .unwrap()
            .is_err());
    }
    fs::remove_file(alias).unwrap();
    let hard = f.root.join("hard.sqlite3");
    fs::hard_link(&f.path, &hard).unwrap();
    assert!(f.open().is_err());
    fs::remove_file(hard).unwrap();
    assert_eq!(fs::read(&f.path).unwrap(), before);
    assert!(f.open().is_ok());
}

#[cfg(unix)]
#[test]
fn changed_or_duplicated_owner_is_not_repaired_or_treated_as_a_fresh_database() {
    let f = Fixture::new();
    f.register();
    f.db.execute("INSERT INTO own_store VALUES('original')", [])
        .unwrap();
    let before = fs::read(&f.path).unwrap();
    assert!(f.open().is_err());
    assert_eq!(fs::read(&f.path).unwrap(), before);
    assert_eq!(
        f.db.query_row("SELECT COUNT(*) FROM own_store", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[cfg(unix)]
#[test]
fn one_registry_cannot_hot_switch_to_another_path_or_copy_but_other_tests_are_isolated() {
    let first = Fixture::new();
    first.register();
    first.register();
    let second = Fixture::new();
    second.register();
    let failure = first.registry.register(
        &second.path,
        &second.db,
        "original".into(),
        Arc::new(|_| Ok(())),
    );
    assert_eq!(failure.unwrap_err(), "service_database_already_registered");
    assert!(first.open().is_ok());
    assert!(second.open().is_ok());
}
