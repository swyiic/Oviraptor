//! Process-wide model request admission. Queue order is FIFO; a waiting stop
//! removes its ticket without issuing a provider request or consuming budget.
use super::gateway::{CancelToken, ModelError};
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, OnceLock},
    time::Duration,
};

pub const CLOUD_MAX_IN_FLIGHT: usize = 3;
const CANCEL_CHECK_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct AdmissionState {
    active: usize,
    next_ticket: u64,
    waiting: VecDeque<u64>,
}

pub struct AdmissionGate {
    capacity: usize,
    state: Mutex<AdmissionState>,
    changed: Condvar,
}

pub struct AdmissionPermit<'a>(&'a AdmissionGate);

impl AdmissionGate {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            capacity,
            state: Mutex::new(AdmissionState::default()),
            changed: Condvar::new(),
        }
    }

    pub fn acquire(&self, cancel: &CancelToken) -> Result<AdmissionPermit<'_>, ModelError> {
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let ticket = state.next_ticket;
        state.next_ticket = state.next_ticket.wrapping_add(1);
        state.waiting.push_back(ticket);
        loop {
            // Do not call the external cancellation checker while holding the
            // queue lock: it may need a database lock held by another waiter.
            drop(state);
            let cancelled = cancel.is_cancelled();
            state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if cancelled {
                state.waiting.retain(|queued| *queued != ticket);
                self.changed.notify_all();
                return Err(ModelError::Cancelled);
            }
            if state.active < self.capacity && state.waiting.front() == Some(&ticket) {
                state.waiting.pop_front();
                state.active += 1;
                return Ok(AdmissionPermit(self));
            }
            state = self
                .changed
                .wait_timeout(state, CANCEL_CHECK_INTERVAL)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .waiting
            .len()
    }
}

impl Drop for AdmissionPermit<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active -= 1;
        self.0.changed.notify_all();
    }
}

static LOCAL: OnceLock<AdmissionGate> = OnceLock::new();
static CLOUD: OnceLock<AdmissionGate> = OnceLock::new();

pub fn gate(local: bool) -> &'static AdmissionGate {
    if local {
        LOCAL.get_or_init(|| AdmissionGate::new(1))
    } else {
        CLOUD.get_or_init(|| AdmissionGate::new(CLOUD_MAX_IN_FLIGHT))
    }
}
