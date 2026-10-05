use rusqlite::{hooks::Wal, Connection};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, LazyLock, Mutex,
};
use std::time::Duration;

// rusqlite's WAL callback is a function pointer, not a capturing closure. This
// process-wide signal contains NO database identity or event data. A commit to
// another database may wake a reader, but that reader only queries its own path.
pub(super) static COMMITS: LazyLock<Arc<CommitWake>> =
    LazyLock::new(|| Arc::new(CommitWake::default()));

#[derive(Default)]
pub(super) struct CommitWake {
    generation: Mutex<u64>,
    changed: Condvar,
}

impl CommitWake {
    pub(super) fn snapshot(&self) -> u64 {
        *self.generation.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn notify(&self) {
        let mut generation = self.generation.lock().unwrap_or_else(|e| e.into_inner());
        *generation = generation.wrapping_add(1);
        self.changed.notify_all();
    }

    pub(super) fn stop(&self, stopped: &AtomicBool) {
        // Synchronize with the waiter entering wait_timeout_while; an atomic
        // flag plus notify without this lock can lose the shutdown wake-up.
        let _guard = self.generation.lock().unwrap_or_else(|e| e.into_inner());
        stopped.store(true, Ordering::Release);
        self.changed.notify_all();
    }

    pub(super) fn wait(&self, observed: Option<u64>, stopped: &AtomicBool, timeout: Duration) {
        let generation = self.generation.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = self
            .changed
            .wait_timeout_while(generation, timeout, |current| {
                !stopped.load(Ordering::Acquire)
                    && match observed {
                        Some(previous) => *current == previous,
                        // Fixed coalescing/backoff delay ignores commits, but
                        // remains stoppable.
                        None => true,
                    }
            })
            .unwrap_or_else(|e| e.into_inner());
    }
}

pub(crate) fn install_commit_notifications(connection: &Connection) {
    connection.wal_hook(Some(on_commit));
}

fn on_commit(wal: &Wal, pages: i32) -> rusqlite::Result<()> {
    COMMITS.notify();
    // Installing a WAL hook replaces SQLite's automatic checkpoint hook. The
    // bundled SQLite/app default is 1000 pages: retain its PASSIVE checkpoint,
    // including ignoring checkpoint errors after an already successful commit.
    // Do not set wal_autocheckpoint later: that would replace this notification.
    if pages >= 1000 {
        let _ = wal.checkpoint();
    }
    Ok(())
}

#[cfg(test)]
#[path = "signal_tests.rs"]
mod tests;
