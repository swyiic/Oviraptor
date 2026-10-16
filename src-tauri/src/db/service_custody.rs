//! Explicit native-process custody. Desktop databases are never registered.
//!
//! One process owns one private store for its entire life. A dropped service,
//! EOF or worker panic cannot make later background opens fall back to CREATE.
use rusqlite::{Connection, OpenFlags};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

type OwnerCheck = dyn Fn(&Connection) -> Result<(), String> + Send + Sync;

pub(crate) struct Custody {
    path: PathBuf,
    original: File,
    identity: (u64, u64),
    owner_id: String,
    owner_check: Arc<OwnerCheck>,
}

fn rejected() -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
        Some("service_database_custody_changed".into()),
    )
}

fn identity(file: &fs::Metadata) -> Result<(u64, u64), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !file.is_file()
            || file.file_type().is_symlink()
            || file.nlink() != 1
            || file.uid() != unsafe { libc::geteuid() }
            || file.mode() & 0o077 != 0
        {
            return Err("service_database_file_permissions".into());
        }
        Ok((file.dev(), file.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Err("service_database_platform_custody_unavailable".into())
    }
}

fn unmoved(handle: usize) -> bool {
    let mut moved: std::os::raw::c_int = 0;
    // No SQL is prepared or executed here. This also runs from the commit
    // hook, where modifying the connection would be forbidden.
    let result = unsafe {
        rusqlite::ffi::sqlite3_file_control(
            handle as *mut rusqlite::ffi::sqlite3,
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
            (&mut moved as *mut std::os::raw::c_int).cast(),
        )
    };
    result == rusqlite::ffi::SQLITE_OK && moved == 0
}

impl Custody {
    fn acquire(
        path: &Path,
        db: &Connection,
        owner_id: String,
        owner_check: Arc<OwnerCheck>,
    ) -> Result<Self, String> {
        if !path.is_absolute()
            || fs::canonicalize(path).map_err(|_| "service_database_path_unavailable")? != path
            || db.path().map(Path::new) != Some(path)
        {
            return Err("service_database_path_invalid".into());
        }
        let expected =
            identity(&fs::symlink_metadata(path).map_err(|_| "service_database_missing")?)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let custody = Self {
            path: path.into(),
            original: options.open(path).map_err(|_| "service_database_missing")?,
            identity: expected,
            owner_id,
            owner_check,
        };
        custody
            .check(db)
            .map_err(|_| "service_database_custody_changed")?;
        Ok(custody)
    }

    fn file_unchanged(&self) -> bool {
        self.original
            .metadata()
            .ok()
            .and_then(|m| identity(&m).ok())
            == Some(self.identity)
            && fs::symlink_metadata(&self.path)
                .ok()
                .and_then(|m| identity(&m).ok())
                == Some(self.identity)
            && fs::canonicalize(&self.path).ok().as_deref() == Some(self.path.as_path())
    }

    fn check(&self, db: &Connection) -> rusqlite::Result<()> {
        let handle = unsafe { db.handle() } as usize;
        if !self.file_unchanged()
            || db.path().map(Path::new) != Some(self.path.as_path())
            || !unmoved(handle)
        {
            return Err(rejected());
        }
        (self.owner_check)(db).map_err(|_| rejected())?;
        if !self.file_unchanged() || !unmoved(handle) {
            return Err(rejected());
        }
        Ok(())
    }

    fn open(self: &Arc<Self>, requested: &Path, flags: OpenFlags) -> rusqlite::Result<Connection> {
        // An alias is not an alternative authority or an invitation to create
        // another store. Canonical service paths are part of original births.
        if requested != self.path || !self.file_unchanged() {
            return Err(rejected());
        }
        let flags = (flags & !OpenFlags::SQLITE_OPEN_CREATE & !OpenFlags::SQLITE_OPEN_URI)
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let db = Connection::open_with_flags(requested, flags)?;
        self.check(&db)?;
        self.attach_commit_guard(&db)?;
        Ok(db)
    }

    fn attach_commit_guard(self: &Arc<Self>, db: &Connection) -> rusqlite::Result<()> {
        let custody = self.clone();
        let handle = unsafe { db.handle() } as usize;
        // Domain authorizers and WAL notification hooks remain independent.
        // Abort a transaction if the opened file moved after the open check.
        db.commit_hook(Some(move || !custody.file_unchanged() || !unmoved(handle)))
    }
}

#[derive(Default)]
struct Registry(Mutex<Option<Arc<Custody>>>);

impl Registry {
    fn register(
        &self,
        path: &Path,
        db: &Connection,
        owner_id: String,
        owner_check: Arc<OwnerCheck>,
    ) -> Result<(), String> {
        let candidate = Arc::new(Custody::acquire(path, db, owner_id, owner_check)?);
        let mut slot = self
            .0
            .lock()
            .map_err(|_| "service_database_registry_unavailable")?;
        if let Some(original) = slot.as_ref() {
            if original.path != candidate.path
                || original.identity != candidate.identity
                || original.owner_id != candidate.owner_id
            {
                return Err("service_database_already_registered".into());
            }
            original
                .check(db)
                .map_err(|_| "service_database_custody_changed")?;
            original
                .attach_commit_guard(db)
                .map_err(|_| "service_database_commit_guard_unavailable")?;
            return Ok(());
        }
        candidate
            .attach_commit_guard(db)
            .map_err(|_| "service_database_commit_guard_unavailable")?;
        *slot = Some(candidate);
        Ok(())
    }

    fn open(&self, path: &Path, flags: OpenFlags) -> Option<rusqlite::Result<Connection>> {
        let slot = match self.0.lock() {
            Ok(slot) => slot,
            Err(_) => return Some(Err(rejected())),
        };
        let custody = slot.as_ref()?.clone();
        drop(slot);
        if path == custody.path
            || fs::canonicalize(path).ok().as_deref() == Some(custody.path.as_path())
        {
            Some(custody.open(path, flags))
        } else {
            None
        }
    }
}

static PROCESS_STORE: LazyLock<Registry> = LazyLock::new(Registry::default);

pub(crate) fn register_process_store(
    path: &Path,
    db: &Connection,
    owner_id: String,
    owner_check: impl Fn(&Connection) -> Result<(), String> + Send + Sync + 'static,
) -> Result<(), String> {
    PROCESS_STORE.register(path, db, owner_id, Arc::new(owner_check))
}

pub(super) fn open_if_registered(
    path: &Path,
    flags: OpenFlags,
) -> Option<rusqlite::Result<Connection>> {
    PROCESS_STORE.open(path, flags)
}

#[cfg(test)]
#[path = "service_custody_tests.rs"]
mod tests;
