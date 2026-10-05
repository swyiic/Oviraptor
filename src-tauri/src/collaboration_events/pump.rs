use super::signal::{CommitWake, COMMITS};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter};

const BATCH_SIZE: usize = 256;
const RECONCILE_INTERVAL: Duration = Duration::from_secs(15);
const ERROR_BACKOFF: Duration = Duration::from_secs(1);
const COMMIT_COALESCE: Duration = Duration::from_millis(25);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CollaborationEventNotification {
    sequence: i64,
    scan_id: String,
    attempt_number: i64,
    entity_type: String,
    entity_id: String,
    event_type: String,
}

pub(crate) struct EventPumpControl {
    stopped: Arc<AtomicBool>,
    commits: Arc<CommitWake>,
}

impl EventPumpControl {
    pub(crate) fn stop(&self) {
        self.commits.stop(&self.stopped);
    }
}

impl Drop for EventPumpControl {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Events are hints, not acknowledgements of UI delivery. Reconnecting clients
/// must still read committed rows using their own durable `afterSequence`.
pub(crate) fn start_event_pump(app: AppHandle, db_path: PathBuf) -> io::Result<EventPumpControl> {
    // Capture before spawning: a delayed worker must not skip new events. If
    // initialization fails, replay hints from zero instead of dropping history.
    let cursor = initial_cursor(&db_path).unwrap_or_else(|_| {
        eprintln!("Collaboration notifications: initial cursor unavailable; replay on recovery");
        0
    });
    let control = EventPumpControl {
        stopped: Arc::new(AtomicBool::new(false)),
        commits: Arc::clone(&COMMITS),
    };
    let stopped = Arc::clone(&control.stopped);
    let commits = Arc::clone(&control.commits);
    thread::Builder::new()
        .name("collaboration-events".into())
        .spawn(move || {
            run(db_path, cursor, commits, stopped, |event| {
                app.emit("nest://collaboration-event", event)
                    .map_err(|error| error.to_string())
            });
        })?;
    // Do not join on the UI thread: an in-flight emit may need that thread.
    Ok(control)
}

fn reader(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_millis(250))
        .map_err(|error| error.to_string())?;
    Ok(connection)
}

fn initial_cursor(path: &Path) -> Result<i64, String> {
    reader(path)?
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn read_batch(path: &Path, cursor: i64) -> Result<Vec<CollaborationEventNotification>, String> {
    let connection = reader(path)?;
    let mut statement = connection
        .prepare(
            "SELECT sequence,scan_id,attempt_number,entity_type,entity_id,event_type \
             FROM agent_collaboration_events WHERE sequence>?1 ORDER BY sequence LIMIT ?2",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(rusqlite::params![cursor, BATCH_SIZE as i64], |row| {
            Ok(CollaborationEventNotification {
                sequence: row.get(0)?,
                scan_id: row.get(1)?,
                attempt_number: row.get(2)?,
                entity_type: row.get(3)?,
                entity_id: row.get(4)?,
                event_type: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn drain_batch(
    path: &Path,
    cursor: &mut i64,
    stopped: &AtomicBool,
    emit: &mut impl FnMut(&CollaborationEventNotification) -> Result<(), String>,
) -> Result<bool, String> {
    let events = read_batch(path, *cursor)?;
    let full = events.len() == BATCH_SIZE;
    for event in events {
        if stopped.load(Ordering::Acquire) {
            return Ok(false);
        }
        emit(&event)?;
        *cursor = event.sequence;
    }
    Ok(full)
}

fn run(
    path: PathBuf,
    mut cursor: i64,
    commits: Arc<CommitWake>,
    stopped: Arc<AtomicBool>,
    mut emit: impl FnMut(&CollaborationEventNotification) -> Result<(), String>,
) {
    let mut warned = false;
    while !stopped.load(Ordering::Acquire) {
        // Snapshot BEFORE the read, closing the commit-between-read-and-wait race.
        let observed = commits.snapshot();
        match drain_batch(&path, &mut cursor, &stopped, &mut emit) {
            Ok(full) => {
                warned = false;
                if full {
                    thread::yield_now();
                } else {
                    wait_for_work(&commits, observed, &stopped);
                }
            }
            Err(_) => {
                if !warned {
                    // Never print event contents, database paths or SQL errors.
                    eprintln!("Collaboration notifications unavailable; retrying retained cursor");
                    warned = true;
                }
                commits.wait(None, &stopped, ERROR_BACKOFF);
            }
        }
    }
}

fn wait_for_work(commits: &CommitWake, observed: u64, stopped: &AtomicBool) {
    // Other processes/unhooked writers require reconciliation. Coalesce bursts
    // after a commit wake: a busy unrelated writer must not create a read spin.
    commits.wait(Some(observed), stopped, RECONCILE_INTERVAL);
    if commits.snapshot() != observed {
        commits.wait(None, stopped, COMMIT_COALESCE);
    }
}

#[cfg(test)]
#[path = "pump_tests.rs"]
mod tests;
