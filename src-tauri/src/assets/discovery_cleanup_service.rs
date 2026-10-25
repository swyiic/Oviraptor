//! One desktop owner for resumable seed research. No work runs on the UI thread.
use super::*;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Default)]
struct Signal {
    stopped: bool,
    armed: bool,
}

fn has_work(path: &Path) -> Result<bool, String> {
    let c = db::open(path)?;
    let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asset_cleanup_runs' AND type='table')",[],|r|r.get(0)).map_err(err)?;
    if !exists {
        return Ok(false);
    }
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM asset_cleanup_runs WHERE enabled=1)",
        [],
        |r| r.get(0),
    )
    .map_err(err)
}

#[derive(Clone)]
pub(crate) struct Manager {
    path: std::path::PathBuf,
    wake: Arc<(Mutex<Signal>, Condvar)>,
}
impl Manager {
    pub(crate) fn new(path: &Path) -> Result<Self, String> {
        let owner = Self {
            path: path.into(),
            wake: Arc::new((
                Mutex::new(Signal {
                    stopped: false,
                    armed: has_work(path)?,
                }),
                Condvar::new(),
            )),
        };
        let worker = owner.clone();
        std::thread::spawn(move || loop {
            let (lock, wake) = &*worker.wake;
            let mut guard = match lock.lock() {
                Ok(guard) => guard,
                Err(_) => break,
            };
            while !guard.stopped && !guard.armed {
                guard = match wake.wait(guard) {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
            }
            if guard.stopped {
                break;
            }
            drop(guard);
            let progressed = match cleanup::tick(&worker.path) {
                Ok(progressed) => progressed,
                Err(error) => {
                    eprintln!(
                        "seed cleanup: {}",
                        crate::commands::exposure::redaction::redact_text(&error)
                    );
                    false
                }
            };
            let mut guard = match lock.lock() {
                Ok(guard) => guard,
                Err(_) => break,
            };
            if guard.stopped {
                break;
            }
            if !progressed {
                guard.armed = has_work(&worker.path).unwrap_or(true);
            }
            if !guard.armed {
                continue;
            }
            // A network request is bounded by its provider deadline; a status
            // request never waits for this worker or starts a second worker.
            let _ = wake.wait_timeout(
                guard,
                Duration::from_millis(if progressed { 100 } else { 2000 }),
            );
        });
        Ok(owner)
    }
    pub(crate) fn dispatch(
        &self,
        p: i64,
        action: &str,
        args: Value,
        operation: &str,
    ) -> Result<Value, String> {
        let result = cleanup::dispatch(&self.path, p, action, args, operation);
        if action != "status" && result.is_ok() {
            if let Ok(mut signal) = self.wake.0.lock() {
                signal.armed = true;
            }
            self.wake.1.notify_all();
        }
        result
    }
    pub(crate) fn shutdown(&self) {
        if let Ok(mut stopped) = self.wake.0.lock() {
            stopped.stopped = true;
        }
        self.wake.1.notify_all();
        let _ = cleanup::interrupt(&self.path);
    }
}

#[tauri::command]
pub async fn seed_cleanup_action(
    manager: tauri::State<'_, Manager>,
    project_id: i64,
    action: String,
    args: Value,
    operation_id: String,
) -> Result<Value, String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        manager.dispatch(project_id, &action, args, &operation_id)
    })
    .await
    .map_err(err)?
}
