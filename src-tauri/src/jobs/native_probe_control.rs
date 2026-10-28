use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

pub(super) struct ProbePacing {
    state: Mutex<(Instant, HashMap<String, Instant>)>,
}

impl ProbePacing {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new((Instant::now(), HashMap::new())),
        }
    }

    pub(super) fn wait(
        &self,
        host: &str,
        rate: f64,
        host_interval: Duration,
        cancel: &AtomicBool,
    ) -> bool {
        let due = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let due = Instant::now()
                .max(state.0)
                .max(state.1.get(host).copied().unwrap_or_else(Instant::now));
            let rate = if rate.is_finite() {
                rate.clamp(0.01, 10_000.0)
            } else {
                1.0
            };
            state.0 = due + Duration::from_secs_f64(1.0 / rate);
            state.1.insert(host.to_string(), due + host_interval);
            due
        };
        while Instant::now() < due {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            std::thread::sleep(
                due.saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(50)),
            );
        }
        !cancel.load(Ordering::Relaxed)
    }
}

