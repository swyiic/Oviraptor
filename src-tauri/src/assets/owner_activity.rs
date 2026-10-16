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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    #[test]
    fn scoped_spawn_tracks_live_barrier_and_preserves_return() {
        let path = PathBuf::from(format!(
            "/synthetic/owner-activity-{}",
            uuid::Uuid::new_v4()
        ));
        let ready = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let a = ready.clone();
        let b = release.clone();
        let worker = with_scope(&path, || {
            thread::spawn(move || {
                a.wait();
                b.wait();
                37
            })
        });
        ready.wait();
        assert_eq!(active_for(&path).unwrap(), 1);
        assert_eq!(
            active_for(Path::new("/synthetic/another-database")).unwrap(),
            0
        );
        release.wait();
        assert_eq!(worker.join().unwrap(), 37);
        assert_eq!(active_for(&path).unwrap(), 0);
    }
    #[test]
    fn scoped_worker_panic_and_nested_scope_release_their_own_counts() {
        let path = PathBuf::from(format!("/synthetic/owner-panic-{}", uuid::Uuid::new_v4()));
        let p = path.clone();
        let worker = with_scope(&path, || {
            thread::spawn(move || {
                let nested = thread::spawn(move || {
                    assert!(active_for(&p).unwrap() >= 1);
                    panic!("synthetic owner worker panic")
                });
                assert!(nested.join().is_err());
            })
        });
        worker.join().unwrap();
        assert_eq!(active_for(&path).unwrap(), 0);
    }
    #[test]
    fn desktop_none_scope_keeps_the_plain_std_spawn_contract() {
        assert_eq!(thread::spawn(|| 11).join().unwrap(), 11);
        let path = PathBuf::from(format!(
            "/synthetic/restored-scope-{}",
            uuid::Uuid::new_v4()
        ));
        with_scope(&path, || {
            assert_eq!(thread::spawn(|| 13).join().unwrap(), 13)
        });
        assert_eq!(active_for(&path).unwrap(), 0);
        assert!(SCOPE.with(|scope| scope.borrow().is_none()));
    }
}
