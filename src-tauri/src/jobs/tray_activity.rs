use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
};

fn animator_owners() -> &'static Mutex<HashMap<usize, Weak<()>>> {
    static OWNERS: OnceLock<Mutex<HashMap<usize, Weak<()>>>> = OnceLock::new();
    OWNERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// There is at most one animator for an AppState's activity counter, including
/// a previous animator that has not woken from its frame delay yet.
pub(super) struct AnimatorLease {
    active_jobs: Arc<AtomicUsize>,
    token: Arc<()>,
}

impl AnimatorLease {
    pub(super) fn acquire(active_jobs: Arc<AtomicUsize>) -> Option<Self> {
        let mut owners = animator_owners()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = Arc::as_ptr(&active_jobs) as usize;
        if owners.get(&key).and_then(Weak::upgrade).is_some() {
            return None;
        }
        let token = Arc::new(());
        owners.insert(key, Arc::downgrade(&token));
        Some(Self { active_jobs, token })
    }

    pub(super) fn should_run(&self) -> bool {
        let mut owners = animator_owners()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.owns(&owners) {
            return false;
        }
        if self.active_jobs.load(Ordering::SeqCst) > 0 {
            return true;
        }
        // The zero check and release share the acquire lock. A new job either
        // keeps this animator alive or acquires its own after this release.
        self.release(&mut owners);
        false
    }

    fn release(&self, owners: &mut HashMap<usize, Weak<()>>) {
        let key = Arc::as_ptr(&self.active_jobs) as usize;
        if self.owns(owners) {
            owners.remove(&key);
        }
    }

    fn owns(&self, owners: &HashMap<usize, Weak<()>>) -> bool {
        owners
            .get(&(Arc::as_ptr(&self.active_jobs) as usize))
            .is_some_and(|owner| owner.as_ptr() == Arc::as_ptr(&self.token))
    }
}

impl Drop for AnimatorLease {
    fn drop(&mut self) {
        let mut owners = animator_owners()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.release(&mut owners);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TrayActivity {
    Idle,
    Updating,
}

impl TrayActivity {
    /// Read when the UI callback runs, not when an animation frame is queued.
    pub(super) fn current(active_jobs: &AtomicUsize) -> Self {
        if active_jobs.load(Ordering::SeqCst) == 0 {
            Self::Idle
        } else {
            Self::Updating
        }
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            // tray-icon's macOS implementation leaves the old title untouched
            // for None. An explicit empty string clears the NSStatusItem title.
            Self::Idle => "",
            Self::Updating => "资产更新中",
        }
    }

    pub(super) fn tooltip(self) -> &'static str {
        match self {
            Self::Idle => "Oviraptor · 后台运行",
            Self::Updating => "Oviraptor · 正在更新资产",
        }
    }
}

/// The ownership of a running job, including a job whose thread cannot start.
/// Dropping it releases activity on success, failure, cancellation and unwind.
pub(super) struct JobActivityGuard {
    active_jobs: Arc<AtomicUsize>,
    on_finished: Option<Box<dyn FnOnce(bool) + Send>>,
}

impl JobActivityGuard {
    pub(super) fn begin(
        active_jobs: Arc<AtomicUsize>,
        on_finished: impl FnOnce(bool) + Send + 'static,
    ) -> (Self, bool) {
        let first_job = active_jobs.fetch_add(1, Ordering::SeqCst) == 0;
        (
            Self {
                active_jobs,
                on_finished: Some(Box::new(on_finished)),
            },
            first_job,
        )
    }
}

impl Drop for JobActivityGuard {
    fn drop(&mut self) {
        let last_job = self.active_jobs.fetch_sub(1, Ordering::SeqCst) == 1;
        if let Some(on_finished) = self.on_finished.take() {
            on_finished(last_job);
        }
    }
}

pub(super) fn catch_job_panic(run: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
        .unwrap_or_else(|_| Err("资产任务线程异常退出，请查看操作日志后重试".into()))
}

