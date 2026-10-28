// Loaded privately by each cohesive Assets domain owner. Each module has
// its OWN registry/TLS; callers must query that same owner's accessor. This is
// thread activity only: no DB, app bootstrap, grants, or admission policy here.
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};
thread_local! { static SCOPE: RefCell<Option<PathBuf>> = const { RefCell::new(None) }; }
static ACTIVE: LazyLock<Mutex<HashMap<PathBuf, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
struct ScopeReset(Option<PathBuf>);
impl Drop for ScopeReset {
    fn drop(&mut self) {
        SCOPE.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}
fn in_scope<F: FnOnce() -> T, T>(scope: Option<PathBuf>, f: F) -> T {
    let previous = SCOPE.with(|slot| slot.replace(scope));
    let _reset = ScopeReset(previous);
    f()
}
pub(super) fn with_scope<F: FnOnce() -> T, T>(path: &Path, f: F) -> T {
    in_scope(Some(path.to_path_buf()), f)
}
pub(super) fn active_for(path: &Path) -> Result<usize, String> {
    ACTIVE
        .lock()
        .map(|counts| counts.get(path).copied().unwrap_or(0))
        .map_err(|_| "asset_owner_activity_unavailable".into())
}
struct Worker(PathBuf);
impl Worker {
    fn new(path: PathBuf) -> Self {
        let mut counts = ACTIVE
            .lock()
            .expect("asset owner activity registry poisoned");
        *counts.entry(path.clone()).or_default() += 1;
        Self(path)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Never introduce a second panic while an accepted worker unwinds.
        // A poisoned registry makes active_for fail closed instead.
        if let Ok(mut counts) = ACTIVE.lock() {
            if let Some(count) = counts.get_mut(&self.0) {
                *count -= 1;
                if *count == 0 {
                    counts.remove(&self.0);
                }
            }
        }
    }
}
pub(super) mod thread {
    // Preserve sleep, Builder, JoinHandle, current, etc. A local explicit spawn
    // takes precedence over the glob import without narrowing std's API.
    pub use std::thread::*;
    pub fn spawn<F, T>(f: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let scope = super::SCOPE.with(|slot| slot.borrow().clone());
        let Some(path) = scope else {
            return std::thread::spawn(f);
        };
        // Captured before std::spawn: its dropped closure releases this guard
        // on OS spawn failure, and the worker releases it on return or panic.
        let worker = super::Worker::new(path.clone());
        std::thread::spawn(move || {
            let _worker = worker;
            super::in_scope(Some(path), f)
        })
    }
}

