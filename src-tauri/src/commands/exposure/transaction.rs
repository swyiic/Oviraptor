// §Q3 — short IMMEDIATE transactions with automatic rollback. The connection is only
// borrowed here, and the network never runs inside a transaction: a claim closes, the fetch
// happens, then a second short transaction records what was seen.

use rusqlite::Error as SqliteError;

pub struct ImmediateTx<'c> {
    connection: &'c rusqlite::Connection,
    finished: bool,
    savepoint: Option<String>,
}

impl<'c> ImmediateTx<'c> {
    /// `BEGIN IMMEDIATE` takes the write lock at once, so two starters serialize instead of
    /// both passing the "is there already an active run?" check. A busy database is retried a
    /// bounded number of times and then reported, never swallowed.
    pub fn begin(connection: &'c rusqlite::Connection) -> Result<Self, String> {
        if !connection.is_autocommit() {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let name = format!(
                "exposure_{}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            connection
                .execute_batch(&format!("SAVEPOINT {name}"))
                .map_err(|error| error.to_string())?;
            return Ok(Self {
                connection,
                finished: false,
                savepoint: Some(name),
            });
        }
        let mut attempts = 0u32;
        loop {
            match connection.execute_batch("BEGIN IMMEDIATE") {
                Ok(()) => {
                    return Ok(Self {
                        connection,
                        finished: false,
                        savepoint: None,
                    })
                }
                Err(error) if busy(&error) && attempts < 4 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(40 * u64::from(attempts)));
                }
                Err(error) => return Err(format!("数据库繁忙，未开始事务：{error}")),
            }
        }
    }

    /// Statements use the normal `rusqlite::params!` macro through this borrow.
    pub fn connection(&self) -> &rusqlite::Connection {
        self.connection
    }

    pub fn insert_row_id(&self) -> i64 {
        self.connection.last_insert_rowid()
    }

    pub fn commit(mut self) -> Result<(), String> {
        self.connection
            .execute_batch(
                &self
                    .savepoint
                    .as_ref()
                    .map(|name| format!("RELEASE {name}"))
                    .unwrap_or_else(|| "COMMIT".into()),
            )
            .map_err(|error| format!("提交失败：{error}"))?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for ImmediateTx<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let sql = self
                .savepoint
                .as_ref()
                .map(|name| format!("ROLLBACK TO {name}; RELEASE {name}"))
                .unwrap_or_else(|| "ROLLBACK".into());
            let _ = self.connection.execute_batch(&sql);
        }
    }
}

fn busy(error: &SqliteError) -> bool {
    let text = error.to_string().to_ascii_lowercase();
    text.contains("database is locked") || text.contains("database is busy")
}

#[cfg(test)]
mod tests {
    use super::ImmediateTx;
    use rusqlite::Connection;
    use std::path::{Path, PathBuf};

    struct TempDb {
        root: PathBuf,
        path: PathBuf,
    }
    impl TempDb {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("ovi-exposure-savepoints-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let path = root.join("atomic.sqlite3");
            Connection::open(&path)
                .unwrap()
                .execute_batch("CREATE TABLE effects (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
                .unwrap();
            Self { root, path }
        }
    }
    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn rows(path: &Path) -> Vec<(i64, String)> {
        let c = Connection::open(path).unwrap();
        let mut statement = c
            .prepare("SELECT id,value FROM effects ORDER BY id")
            .unwrap();
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    }

    #[test]
    fn outer_rollback_reverts_released_nested_savepoint_writes() {
        let db = TempDb::new();
        let c = Connection::open(&db.path).unwrap();
        {
            let outer = ImmediateTx::begin(&c).unwrap();
            outer
                .connection()
                .execute("INSERT INTO effects VALUES(1,'outer evidence')", [])
                .unwrap();
            let inner = ImmediateTx::begin(outer.connection()).unwrap();
            inner
                .connection()
                .execute("INSERT INTO effects VALUES(2,'nested ledger')", [])
                .unwrap();
            inner.commit().unwrap();
            assert!(
                !c.is_autocommit(),
                "releasing a savepoint cannot commit the outer transaction"
            );
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM effects", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                2
            );
            assert!(
                rows(&db.path).is_empty(),
                "another connection must not see either uncommitted write"
            );
            // Dropping the outer transaction models failure after the nested operation
            // succeeded. Evidence and nested ledger changes must roll back together.
        }
        assert!(c.is_autocommit());
        assert!(
            rows(&db.path).is_empty(),
            "released nested writes still belong to the outer atomic unit"
        );
    }

    #[test]
    fn dropped_nested_savepoint_rolls_back_only_itself_and_outer_can_commit() {
        let db = TempDb::new();
        let c = Connection::open(&db.path).unwrap();
        let outer = ImmediateTx::begin(&c).unwrap();
        outer
            .connection()
            .execute("INSERT INTO effects VALUES(1,'before nested failure')", [])
            .unwrap();
        {
            let inner = ImmediateTx::begin(outer.connection()).unwrap();
            inner
                .connection()
                .execute("INSERT INTO effects VALUES(2,'failed nested write')", [])
                .unwrap();
            // The inner operation failed and is dropped without commit.
        }
        assert!(
            !c.is_autocommit(),
            "inner rollback must leave the outer transaction open"
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM effects WHERE id=2", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        outer
            .connection()
            .execute("INSERT INTO effects VALUES(3,'after nested failure')", [])
            .unwrap();
        outer.commit().unwrap();
        assert!(c.is_autocommit());
        assert_eq!(
            rows(&db.path),
            vec![
                (1, "before nested failure".into()),
                (3, "after nested failure".into())
            ]
        );
    }

    #[test]
    fn top_level_commit_persists_all_writes_and_returns_to_autocommit() {
        let db = TempDb::new();
        let c = Connection::open(&db.path).unwrap();
        assert!(c.is_autocommit());
        let tx = ImmediateTx::begin(&c).unwrap();
        assert!(!c.is_autocommit());
        tx.connection()
            .execute("INSERT INTO effects VALUES(1,'durable evidence')", [])
            .unwrap();
        tx.connection()
            .execute("INSERT INTO effects VALUES(2,'durable usage')", [])
            .unwrap();
        assert!(
            rows(&db.path).is_empty(),
            "ordinary transaction writes remain private before commit"
        );
        tx.commit().unwrap();
        assert!(c.is_autocommit());
        drop(c);
        assert_eq!(
            rows(&db.path),
            vec![(1, "durable evidence".into()), (2, "durable usage".into())]
        );
    }
}
