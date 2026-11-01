use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const DEFAULT_BUSINESS_FRONTEND_SKILL: &str =
    include_str!("../resources/skills/business_frontend_deep_analysis.md");

#[path = "db/service_custody.rs"]
pub(crate) mod service_custody;

/// Preserve each private writer's flags and policy while applying a registered
/// native service's process-lifetime database custody before any SQL runs.
pub(crate) fn open_with_flags(path: impl AsRef<Path>, flags: OpenFlags) -> rusqlite::Result<Connection> {
    let path = path.as_ref();
    match service_custody::open_if_registered(path, flags) {
        Some(result) => result,
        None => Connection::open_with_flags(path, flags),
    }
}

pub fn open(path: &Path) -> Result<Connection, String> {
    let connection = open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| error.to_string())?;
    // journal_mode is persistent and is enabled once during initialize().
    // Re-applying it for every UI command asks SQLite for a write lock. While a
    // collector is writing this made otherwise read-only page changes wait for
    // the full busy timeout and looked like the application had frozen.
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| error.to_string())?;
    crate::collaboration_events::install_commit_notifications(&connection);
    Ok(connection)
}

/// True for a transient SQLite lock/busy error — the kind that clears if you
/// wait and retry, as opposed to a genuine persistence or schema failure. Under
/// WAL, SQLite still allows only one writer at a time; when several multi-agent
/// writers race for `BEGIN IMMEDIATE`, the losers wait out `busy_timeout` and
/// then surface one of these. Such an error must NOT be escalated to a
/// scan-killing terminal — it is retryable.
pub(crate) fn is_transient_lock(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    // The single source of truth for transient-lock detection, shared by the scan
    // engine and the asset subsystem (which previously each had their own
    // `storage_busy`). `contains` (not `starts_with`) so a wrapped message like
    // "SQLITE_BUSY: database is locked" still matches.
    [
        "database is locked",
        "database table is locked",
        "database schema is locked",
        "database is busy",
        "sqlite_busy",
        "sqlite_locked",
    ]
    .iter()
    .any(|token| error.contains(token))
}

/// True for a transient *contention* condition — a superset of `is_transient_lock`
/// that also covers the multi-agent budget-reservation collisions that are "no
/// capacity / stale lease right now", not a real failure: a child budget
/// reservation that was momentarily squeezed out by a large concurrent tick
/// reservation (`child_budget_reservation_exceeded_or_stale`) or an executor
/// grant that found no budget window at read time
/// (`multi_agent_executor_budget_unavailable`). In both the reservation never
/// committed, so nothing was written and nothing was charged — the attempt must
/// stay recoverable (resumable), never a fatal persistence failure that discards
/// already-gathered recon. This deliberately does NOT include
/// `budget_indeterminate_requires_reconciliation`, which is a real reconciliation
/// safety signal, nor a genuine budget-exhaustion stop.
pub(crate) fn is_transient_contention(error: &str) -> bool {
    is_transient_lock(error)
        || error.contains("child_budget_reservation_exceeded_or_stale")
        || error.contains("multi_agent_executor_budget_unavailable")
}

/// Run a write operation, retrying on a transient lock with bounded exponential
/// backoff before giving up. The operation MUST be idempotent across a retry:
/// a transaction either committed atomically or rolled back, so re-running after
/// a lock abort is safe. A non-lock error returns immediately, unchanged.
pub(crate) fn with_write_retry<T>(
    mut op: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    const ATTEMPTS: usize = 5;
    let mut delay = Duration::from_millis(100);
    let mut last = String::new();
    for attempt in 1..=ATTEMPTS {
        match op() {
            Ok(value) => return Ok(value),
            Err(error) if is_transient_lock(&error) && attempt < ATTEMPTS => {
                last = error;
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_secs(2));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last)
}

/// Process-wide per-database write-serialization locks. SQLite allows only one
/// writer at a time; serializing the hottest multi-agent writers through this
/// in-process lock makes them queue instead of colliding at the SQLite layer and
/// timing out. The guard MUST wrap only a short, self-contained write and must
/// never be held across spawning a child, awaiting, or re-acquiring the guard
/// (std `Mutex` is not reentrant) — otherwise it would deadlock. It is a
/// best-effort serializer, not a correctness boundary: writers that do not take
/// it still contend, which is why `with_write_retry` remains the backstop.
fn write_locks() -> &'static std::sync::Mutex<
    std::collections::HashMap<PathBuf, std::sync::Arc<std::sync::Mutex<()>>>,
> {
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Arc<std::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    LOCKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

thread_local! {
    /// Which database paths this thread already holds the write guard for, so a
    /// guarded write nested inside another guarded write (same thread) re-enters
    /// instead of deadlocking. This is what makes it safe to place the guard at
    /// many call sites, including ones that call each other.
    static HELD_WRITE_LOCKS: std::cell::RefCell<Vec<PathBuf>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn write_lock_for(path: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    let mut map = write_locks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.entry(path.to_path_buf())
        .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(())))
        .clone()
}

/// Run `op` while holding the per-database write-serialization guard for `path`.
///
/// Reentrant per thread: if this thread already holds the guard for `path`, `op`
/// runs immediately without re-locking, so nested guarded writes never deadlock.
/// Across threads it serializes — a second thread waits until the first releases.
///
/// The guard MUST still wrap only a short, DB-only write section. Never hold it
/// across a model/network call, a child spawn, or any blocking wait, or every
/// other writer stalls behind that call. Combine with `with_write_retry` (or use
/// `guarded_write`) so a collision with a writer that does NOT take the guard is
/// still survived.
pub(crate) fn with_write_lock<T>(path: &Path, op: impl FnOnce() -> T) -> T {
    let already_held =
        HELD_WRITE_LOCKS.with(|held| held.borrow().iter().any(|p| p.as_path() == path));
    if already_held {
        return op();
    }
    let lock = write_lock_for(path);
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    HELD_WRITE_LOCKS.with(|held| held.borrow_mut().push(path.to_path_buf()));
    // Pop the held-marker on the way out, even on panic, before the mutex guard
    // is released (locals drop in reverse order, so declare this after `_guard`).
    struct Unmark(PathBuf);
    impl Drop for Unmark {
        fn drop(&mut self) {
            HELD_WRITE_LOCKS.with(|held| {
                let mut held = held.borrow_mut();
                if let Some(index) = held.iter().rposition(|p| p == &self.0) {
                    held.remove(index);
                }
            });
        }
    }
    let _unmark = Unmark(path.to_path_buf());
    op()
}

/// Serialize AND survive: hold the write guard for `path` while running `op`,
/// retrying `op` on a transient lock. The one-call form of the pattern used at
/// every guarded write site. `op` must be idempotent across a retry.
pub(crate) fn guarded_write<T>(
    path: &Path,
    op: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    with_write_lock(path, || with_write_retry(op))
}

fn environment_preparation_lock(path: &Path) -> Result<fs::File, String> {
    let lock_path = path.with_extension("preparation.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("无法打开环境准备锁 {}：{error}", lock_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("无法保护环境准备锁：{error}"))?;
    }
    Ok(file)
}

/// Bounded wait for references to a just-released preparation lock held only
/// by children that another thread is spawning right now. std opens the lock
/// file with O_CLOEXEC, but a spawned child shares every open file description
/// from fork/spawn until exec closes it, so for a few milliseconds a released
/// flock still reads as held (measured on macOS with 4 concurrent spawners:
/// ~3% of immediate re-locks failed). A real preparation holds the lock for the
/// whole install, far longer than this grace, so it is still refused.
const PREPARATION_LOCK_SPAWN_SETTLE: Duration = Duration::from_millis(200);

fn try_lock_settled(file: &fs::File) -> Result<(), fs::TryLockError> {
    let deadline = Instant::now() + PREPARATION_LOCK_SPAWN_SETTLE;
    loop {
        match file.try_lock() {
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            result => return result,
        }
    }
}

/// The OS lock is released on process death. The SQLite row is the durable
/// fence seen by every scan activation, including other application processes.
pub struct EnvironmentPreparationLease {
    db_path: PathBuf,
    owner: String,
    _lock: fs::File,
    #[cfg(unix)]
    original_database: fs::File,
}

impl EnvironmentPreparationLease {
    /// Authority comes from this held OS guard and its exact durable owner,
    /// never from the mere presence of a preparation row.
    pub(crate) fn require_tool_supply_on(
        &self,
        tx: &rusqlite::Transaction<'_>,
    ) -> Result<(), String> {
        let rejected = || "tool_supply_preparation_owner_mismatch".to_string();
        let path = tx.path().filter(|path| !path.is_empty()).ok_or_else(rejected)?;
        if fs::canonicalize(path).map_err(|_| rejected())?
            != fs::canonicalize(&self.db_path).map_err(|_| rejected())?
        {
            return Err(rejected());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let original = self.original_database.metadata().map_err(|_| rejected())?;
            let database = fs::metadata(path).map_err(|_| rejected())?;
            if original.dev() != database.dev() || original.ino() != database.ino() {
                return Err(rejected());
            }
            // Ask SQLite about its opened entity too: checking the current
            // pathname alone misses a connection still holding a renamed DB.
            let mut moved: std::os::raw::c_int = 0;
            let checked = unsafe {
                rusqlite::ffi::sqlite3_file_control(
                    tx.handle(), c"main".as_ptr(),
                    rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
                    (&mut moved as *mut std::os::raw::c_int).cast(),
                )
            };
            if checked != rusqlite::ffi::SQLITE_OK || moved != 0 {
                return Err(rejected());
            }
            let held = self._lock.metadata().map_err(|_| rejected())?;
            let current = fs::symlink_metadata(self.db_path.with_extension("preparation.lock"))
                .map_err(|_| rejected())?;
            if !current.is_file() || current.file_type().is_symlink()
                || held.dev() != current.dev() || held.ino() != current.ino()
            {
                return Err(rejected());
            }
        }
        let shadowed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name IN ('environment_preparation_lease','sentinel_scans'))",
            [], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if shadowed { return Err(rejected()); }
        let owns: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.environment_preparation_lease WHERE singleton=1 AND owner=?1) AND NOT EXISTS(SELECT 1 FROM main.sentinel_scans WHERE status IN ('queued','scanning','pausing'))",
            [&self.owner], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if !owns { return Err(rejected()); }
        Ok(())
    }

    /// An inert tool candidate is not an approved or runnable capability. Log
    /// its exact digest and identity key while this process still owns the
    /// preparation fence. Different signed releases may share package bytes.
    pub(crate) fn record_tool_candidate(
        &self,
        tool_id: &str,
        digest: &str,
        identity_key: &str,
        already_present: bool,
    ) -> Result<(), String> {
        let event = format!(
            "tool_candidate_{}:{tool_id}:{digest}:{identity_key}",
            if already_present {
                "revalidated"
            } else {
                "staged"
            }
        );
        let connection = open(&self.db_path)?;
        let inserted = connection
            .execute(
                "INSERT INTO environment_preparation_events(owner,event) SELECT owner,?2 FROM environment_preparation_lease WHERE singleton=1 AND owner=?1",
                params![self.owner, event],
            )
            .map_err(|error| error.to_string())?;
        if inserted != 1 {
            return Err("环境准备租约已变化，不能记录能力包候选审计".into());
        }
        Ok(())
    }

    /// Only the caller that observed a complete preparation may release the
    /// durable fence. Dropping the guard (including unwinding) releases the OS
    /// lock but deliberately leaves the row for manual recovery.
    pub fn complete(self) -> Result<(), String> {
        let mut connection = open(&self.db_path)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        self.require_tool_supply_on(&tx)?;
        let removed = tx
            .execute(
                "DELETE FROM environment_preparation_lease WHERE singleton=1 AND owner=?1",
                [&self.owner],
            )
            .map_err(|error| error.to_string())?;
        if removed != 1 {
            return Err("环境准备租约已变化，不能标记为完成".into());
        }
        tx.execute(
            "INSERT INTO environment_preparation_events(owner,event) VALUES(?1,'completed')",
            [&self.owner],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentPreparationStatus {
    pub state: &'static str,
    pub owner: Option<String>,
    pub created_at: Option<String>,
}

/// A free OS lock is not evidence that an installer child has exited. Keep
/// the durable fence visible until an operator explicitly recovers it.
pub fn environment_preparation_status(path: &Path) -> Result<EnvironmentPreparationStatus, String> {
    let lock = environment_preparation_lock(path)?;
    let lock_available = try_lock_settled(&lock).is_ok();
    let connection = open(path)?;
    let lease: Option<(String, String)> = connection
        .query_row(
            "SELECT owner,created_at FROM environment_preparation_lease WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let (owner, created_at) = match lease {
        Some((owner, created_at)) => (Some(owner), Some(created_at)),
        None => (None, None),
    };
    Ok(EnvironmentPreparationStatus {
        state: if !lock_available {
            "installing"
        } else if owner.is_some() {
            "requires_manual_recovery"
        } else {
            "idle"
        },
        owner,
        created_at,
    })
}

/// The acknowledgement is a human attestation, not a process-verification
/// mechanism. Recovery is refused while the OS lock is held or scans are live.
pub fn recover_environment_preparation(
    path: &Path,
    expected_owner: &str,
    acknowledgement: &str,
) -> Result<(), String> {
    if expected_owner.is_empty() || acknowledgement != "已确认安装子进程停止" {
        return Err("需核对租约并输入完整确认语，确认所有安装子进程已经停止".into());
    }
    let lock = environment_preparation_lock(path)?;
    try_lock_settled(&lock).map_err(|_| "环境准备仍在运行，不能解除锁定".to_string())?;
    let mut connection = open(path)?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let active: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE status IN ('queued','scanning','pausing'))",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if active != 0 {
        return Err("仍有活跃或排队的扫描，不能恢复环境准备租约".into());
    }
    let removed = tx
        .execute(
            "DELETE FROM environment_preparation_lease WHERE singleton=1 AND owner=?1",
            [expected_owner],
        )
        .map_err(|error| error.to_string())?;
    if removed != 1 {
        return Err("环境准备租约已变化；请重新获取状态后核对".into());
    }
    tx.execute(
        "INSERT INTO environment_preparation_events(owner,event) VALUES(?1,'manual_recovered_installer_verified')",
        [expected_owner],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())
}

/// Claim preparation and check the queue in one SQLite writer transaction.
/// No task can enter queued/scanning until this guard is completed. A process
/// crash leaves the durable row fenced: child installers may outlive us, so an
/// absent OS lock alone is insufficient proof that it is safe to recover.
pub fn begin_environment_preparation(path: &Path) -> Result<EnvironmentPreparationLease, String> {
    let lock = environment_preparation_lock(path)?;
    try_lock_settled(&lock)
        .map_err(|error| format!("已有环境准备正在进行，拒绝同时安装：{error}"))?;
    let mut connection = open(path)?;
    #[cfg(unix)]
    let original_database = {
        use std::os::unix::fs::MetadataExt;
        let file = fs::File::open(path).map_err(|error| error.to_string())?;
        let held = file.metadata().map_err(|error| error.to_string())?;
        let current = fs::metadata(path).map_err(|error| error.to_string())?;
        if held.dev() != current.dev() || held.ino() != current.ino() {
            return Err("environment_preparation_database_changed".into());
        }
        file
    };
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let active: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE status IN ('queued','scanning','pausing'))",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if active != 0 {
        return Err(
            "存在等待或正在执行的扫描任务；请先完成或暂停并移出队列，再由管理员安装环境依赖".into(),
        );
    }
    if tx
        .query_row(
            "SELECT owner FROM environment_preparation_lease WHERE singleton=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err(
            "上次环境准备未正常结束；可能仍有安装子进程，拒绝新任务与再次安装，需管理员核对后恢复"
                .into(),
        );
    }
    let owner = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO environment_preparation_lease(singleton,owner) VALUES(1,?1)",
        [&owner],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO environment_preparation_events(owner,event) VALUES(?1,'started')",
        [&owner],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(EnvironmentPreparationLease {
        db_path: path.to_path_buf(),
        owner,
        _lock: lock,
        #[cfg(unix)]
        original_database,
    })
}

#[path = "db_settings.rs"]
mod settings_migration;
pub use settings_migration::normalize_settings;
#[path = "agent_runtime/multi_agent/attempts/schema.rs"]
mod assignment_attempt_schema;
#[path = "agent_runtime/multi_agent/budget/schema.rs"]
mod budget_schema;

pub fn settings_for_update(
    stored: &serde_json::Value,
    incoming: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    settings_migration::settings_for_update(stored, incoming)
}

pub fn initialize(app_data_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(app_data_dir).map_err(|error| error.to_string())?;
    let path = app_data_dir.join("oviraptor.sqlite3");
    let mut connection = open(&path)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    connection
        .execute_batch(SCHEMA)
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(budget_schema::SCHEMA)
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("agent_runtime/web_mode/schema.sql"))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("commands/native_protection_recovery_schema.sql"))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("commands/native_protection_recovery_priced_schema.sql"))
        .map_err(|error| error.to_string())?;
    connection.execute_batch(include_str!("agent_runtime/web_mode/root/birth_seal.sql"))
        .map_err(|e| e.to_string())?;
    connection
        .execute_batch(include_str!("agent_runtime/multi_agent/budget/root/model/tick/schema.sql"))
        .map_err(|error|error.to_string())?;
    connection
        .execute_batch(include_str!("agent_runtime/multi_agent/budget/root/model/tick/confirmation/schema.sql"))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(assignment_attempt_schema::SCHEMA)
        .map_err(|error| error.to_string())?;
    connection.execute_batch(include_str!("agent_runtime/multi_agent/attempts/withdrawal_receipt/schema.sql"))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("native_pipeline/process_log/schema.sql"))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!(
            "agent_runtime/multi_agent/directive/human_review/schema.sql"
        ))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!(
            "agent_runtime/multi_agent/directive/proposals/owned/schema.sql"
        ))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(include_str!("agent_runtime/model/diagnostics/schema.sql"))
        .map_err(|error| error.to_string())?;
    connection.execute_batch(include_str!("agent_runtime/deleted_scan_audit/schema.sql")).map_err(|e|e.to_string())?;
    connection.execute_batch(include_str!("agent_runtime/multi_agent/budget/clock/finalization/exit_receipt/schema.sql")).map_err(|e|e.to_string())?;
    connection.execute_batch(crate::agent_runtime::multi_agent::specialist::mapper_loop::SCHEMA)
        .map_err(|e|format!("无法初始化 Mapper 多轮原费用账据：{e}"))?;
    connection.execute_batch(crate::agent_runtime::multi_agent::specialist::mapper_loop::final_seal::FINAL_SCHEMA)
        .map_err(|e|format!("无法初始化 Mapper 多轮最终原收据：{e}"))?;
    connection.execute_batch(crate::agent_runtime::multi_agent::specialist::mapper_loop::expired_seal::EXPIRED_SCHEMA)
        .map_err(|e|format!("无法初始化 Mapper 过期原输出收据：{e}"))?;
    connection.execute_batch(include_str!("tool_supply/approval/schema.sql"))
        .map_err(|e|format!("无法初始化工具管理员批准库存：{e}"))?;
    connection.execute_batch(include_str!("tool_supply/frozen_plan/schema.sql"))
        .map_err(|e|format!("无法初始化任务工具冻结计划：{e}"))?;
    connection.execute_batch(include_str!("tool_supply/selection/schema.sql"))
        .map_err(|e|format!("无法初始化新草稿工具准备选择：{e}"))?;
    // Existing runs have unknown coverage. Do not fabricate completed receipts
    // or reinterpret their historical terminal state during this additive migration.
    ensure_column(&connection, "runs", "collection_summary_json", "TEXT NOT NULL DEFAULT ''")?;
    // Unknown historical scopes cannot establish absence under a new query plan.
    ensure_column(&connection, "project_assets", "last_collection_binding", "TEXT NOT NULL DEFAULT ''")?;
    crate::asset_logs::initialize(&connection)?;
    migrate_import_membership_source_paths(&connection)?;
    suspend_collaboration_event_triggers(&connection)?;
    migrate_legacy_agent_and_retry_columns(&mut connection)?;
    verify_locked_platform_schema(&mut connection)?;
    // Live knowledge uses neutral tables. Startup must not import or mutate retired
    // backend rows: old data is not part of the Native runtime contract.
    connection.execute_batch(include_str!("agent_runtime/multi_agent/directive/ordered_execution/schema.sql"))
        .map_err(|e|format!("无法初始化有序评估回执：{e}"))?;
    connection
        .execute_batch(NEUTRAL_KNOWLEDGE_SCHEMA)
        .map_err(|error| error.to_string())?;
    // Register the Skill package schema only. Bundled resources are installed
    // lazily when the Skills manager is opened, never during Chat startup.
    crate::skill_packages::initialize(&mut connection)?;
    migrate_agent_run_orchestration_columns(&mut connection)?;
    worker_namespaces::migrate(&mut connection)?;
    migrate_evidence_revision_schema(&connection)?;
    migrate_agent_review_verdict_vocabulary(&mut connection)?;
    migrate_fuse_entry_status(&mut connection)?;
    migrate_builtin_prompts_and_recon_routes(&mut connection)?;
    seed_builtin_agent_skill(&mut connection)?;
    migrate_targets_and_opportunities(&mut connection)?;
    reconcile_investigation_source_keys(&mut connection)?;
    repair_asset_duplicates_and_probe_labels(&mut connection)?;
    migrate_budget_defaults(&mut connection)?;
    repair_legacy_foreign_keys(&mut connection)?;
    crate::collaboration_events::ensure_schema(&connection)?;
    connection.execute_batch(include_str!("agent_runtime/multi_agent/budget/root/model/tick/timeline_event_schema.sql"))
        .map_err(|error| error.to_string())?;
    // §9.3 agent stage-adjudication timeline. Purely additive (one new table +
    // index); records what the agent decided per stage, grants no authority.
    connection
        .execute_batch(crate::scanning::agent_stage_decisions::SCHEMA)
        .map_err(|error| error.to_string())?;
    // Per-scan source-fetch consent (the task-creation "allow source fetch"
    // gate). Additive; default-deny. See scanning::source_collection.
    connection
        .execute_batch(crate::scanning::source_collection::CONSENT_SCHEMA)
        .map_err(|error| error.to_string())?;
    Ok(path)
}

/// A content-addressed bundle may be imported from several directories. The
/// bundle's latest source_path cannot identify the source of older memberships.
/// Existing rows can only be backfilled when their bundle has a single scope.
/// For an ambiguous old hash shared by multiple scopes, leave the path empty
/// rather than misattribute one scan's historical claims to another scan.
/// New rows capture the exact path when the projection is committed.
fn migrate_import_membership_source_paths(connection: &Connection) -> Result<(), String> {
    ensure_column(
        connection,
        "import_projection_memberships",
        "source_path",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    connection.execute(
        "UPDATE import_projection_memberships SET source_path=(\
            SELECT b.source_path FROM import_bundles b WHERE b.id=import_projection_memberships.bundle_row_id\
         ) WHERE source_path='' AND bundle_row_id IN (
            SELECT bundle_row_id FROM import_projection_memberships
            GROUP BY bundle_row_id HAVING COUNT(DISTINCT scope_key)=1
         )",
        [],
    ).map_err(|error| format!("无法迁移历史投影来源目录：{error}"))?;
    Ok(())
}

// §12: the migration helpers, the schema text and the tests live in their own
// files. These are textual includes of the same module, so nothing had to be made
// public or duplicated.
include!("db_migrate.rs");
include!("db_schema.rs");
include!("db_initialize.rs");
#[path = "db/worker_namespaces.rs"]
mod worker_namespaces;
include!("db_neutral.rs");


#[cfg(test)]
mod write_lock_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn is_transient_lock_matches_busy_and_locked_only() {
        assert!(is_transient_lock("database is locked"));
        assert!(is_transient_lock("SQLITE_BUSY: database is locked"));
        assert!(is_transient_lock("database table is locked"));
        assert!(is_transient_lock("database schema is locked"));
        assert!(is_transient_lock("database is busy"));
        assert!(!is_transient_lock("UNIQUE constraint failed"));
        assert!(!is_transient_lock("no such table: x"));
        assert!(!is_transient_lock("budget_indeterminate_requires_reconciliation"));
    }

    #[test]
    fn is_transient_contention_covers_locks_and_transient_reservations() {
        // Superset of locks: the transient budget-capacity collisions.
        assert!(is_transient_contention("database is locked"));
        assert!(is_transient_contention("child_budget_reservation_exceeded_or_stale"));
        assert!(is_transient_contention("multi_agent_executor_budget_unavailable"));
        // Must NOT swallow a real reconciliation signal or a genuine error.
        assert!(!is_transient_contention("budget_indeterminate_requires_reconciliation"));
        assert!(!is_transient_contention("UNIQUE constraint failed"));
    }

    #[test]
    fn with_write_retry_recovers_after_transient_locks() {
        let attempts = Cell::new(0);
        let result = with_write_retry(|| {
            attempts.set(attempts.get() + 1);
            if attempts.get() < 3 {
                Err("database is locked".to_string())
            } else {
                Ok(attempts.get())
            }
        });
        assert_eq!(result, Ok(3));
    }

    #[test]
    fn with_write_retry_passes_non_lock_errors_through_immediately() {
        let attempts = Cell::new(0);
        let result: Result<(), String> = with_write_retry(|| {
            attempts.set(attempts.get() + 1);
            Err("UNIQUE constraint failed".to_string())
        });
        assert_eq!(result, Err("UNIQUE constraint failed".to_string()));
        assert_eq!(attempts.get(), 1, "a real error must not be retried");
    }

    #[test]
    fn with_write_lock_serializes_and_returns_value() {
        let path = Path::new("/tmp/oviraptor-write-lock-test.sqlite3");
        let value = with_write_lock(path, || 42);
        assert_eq!(value, 42);
    }

    #[test]
    fn with_write_lock_is_reentrant_on_same_thread() {
        let path = Path::new("/tmp/oviraptor-reentrant-test.sqlite3");
        // A guarded write nested inside another on the same thread must run, not
        // deadlock on the non-recursive underlying mutex.
        let result = with_write_lock(path, || with_write_lock(path, || 7));
        assert_eq!(result, 7);
        // The held-marker is fully unwound after both frames exit.
        let still_held = HELD_WRITE_LOCKS.with(|held| {
            held.borrow().iter().any(|p| p.as_path() == path)
        });
        assert!(!still_held, "reentrant guard must not leak a held-marker");
    }

    #[test]
    fn guarded_write_runs_and_returns() {
        let path = Path::new("/tmp/oviraptor-guarded-write-test.sqlite3");
        let value = guarded_write(path, || Ok::<_, String>(99));
        assert_eq!(value, Ok(99));
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "foreign_keys", "ON").unwrap();
        connection
    }

    #[test]
    fn rebuild_keeps_child_rows_and_their_foreign_key_target() {
        let mut c = connection();
        c.execute_batch(
            "CREATE TABLE parent(id TEXT PRIMARY KEY, status TEXT NOT NULL CHECK(status IN ('a')));
             CREATE TABLE child(id TEXT PRIMARY KEY REFERENCES parent(id) ON DELETE CASCADE);
             INSERT INTO parent VALUES('p1','a'); INSERT INTO child VALUES('p1');",
        ).unwrap();
        rebuild_table(&mut c, "parent", "
            CREATE TABLE parent__rebuild(id TEXT PRIMARY KEY, status TEXT NOT NULL CHECK(status IN ('a','b')));
            INSERT INTO parent__rebuild SELECT id,status FROM parent;
            DROP TABLE parent;
            ALTER TABLE parent__rebuild RENAME TO parent;").unwrap();
        let children: i64 = c.query_row("SELECT COUNT(*) FROM child", [], |r| r.get(0)).unwrap();
        assert_eq!(children, 1, "the child row must survive the rebuild");
        let child_sql: String = c.query_row("SELECT sql FROM sqlite_master WHERE name='child'", [], |r| r.get(0)).unwrap();
        assert!(child_sql.contains("REFERENCES parent(id)"), "{child_sql}");
        c.execute("INSERT INTO parent VALUES('p2','b')", []).unwrap();
        c.execute("INSERT INTO child VALUES('p2')", []).unwrap();
        let foreign_keys: i64 = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn failed_rebuild_leaves_the_original_table() {
        let mut c = connection();
        c.execute_batch("CREATE TABLE parent(id TEXT PRIMARY KEY); INSERT INTO parent VALUES('p1');").unwrap();
        let error = rebuild_table(&mut c, "parent", "
            CREATE TABLE parent__rebuild(id TEXT PRIMARY KEY, extra TEXT NOT NULL);
            INSERT INTO parent__rebuild SELECT id, NULL FROM parent;
            DROP TABLE parent;
            ALTER TABLE parent__rebuild RENAME TO parent;").unwrap_err();
        assert!(error.contains("NOT NULL"), "{error}");
        let rows: i64 = c.query_row("SELECT COUNT(*) FROM parent", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1);
        let rebuilt: i64 = c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='parent__rebuild'", [], |r| r.get(0)).unwrap();
        assert_eq!(rebuilt, 0);
    }

    #[test]
    fn repair_points_orphaned_foreign_keys_back_at_the_parent() {
        let mut c = connection();
        c.execute_batch(
            "CREATE TABLE agent_review_requests(id TEXT PRIMARY KEY);
             CREATE TABLE agent_gap_review_receipts(
                request_id TEXT PRIMARY KEY REFERENCES \"agent_review_requests_legacy_verdict\"(id) ON DELETE CASCADE,
                note TEXT NOT NULL DEFAULT '');
             CREATE INDEX idx_receipts_note ON agent_gap_review_receipts(note);
             INSERT INTO agent_review_requests VALUES('r1');",
        ).unwrap();
        repair_legacy_foreign_keys(&mut c).unwrap();
        let sql: String = c.query_row("SELECT sql FROM sqlite_master WHERE name='agent_gap_review_receipts'", [], |r| r.get(0)).unwrap();
        assert!(!sql.contains("legacy"), "{sql}");
        c.execute("INSERT INTO agent_gap_review_receipts(request_id) VALUES('r1')", []).unwrap();
        let index: i64 = c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='idx_receipts_note'", [], |r| r.get(0)).unwrap();
        assert_eq!(index, 1, "dependent indexes are recreated");
        repair_legacy_foreign_keys(&mut c).unwrap();
    }
}
