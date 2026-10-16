//! Diagnostic worktree only. Root-scoped test observations never grant rights,
//! alter cancellation decisions, change clocks, or run in production builds.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Debug)]
pub(crate) struct Observation {
    pub(crate) branch: String,
    pub(crate) detail: String,
}
type ElapsedHook = Box<dyn FnOnce(Option<i64>) + Send>;
type GatewayHook = Box<dyn FnOnce(&rusqlite::Connection) + Send>;
#[derive(Default)]
struct Hooks {
    elapsed: Mutex<Option<ElapsedHook>>,
    gateway: Mutex<Option<GatewayHook>>,
    events: Mutex<Vec<Observation>>,
}
static ROOTS: OnceLock<Mutex<HashMap<String, Arc<Hooks>>>> = OnceLock::new();
fn hooks(root: &str) -> Option<Arc<Hooks>> {
    ROOTS.get()?.lock().ok()?.get(root).cloned()
}
pub(crate) struct Installed {
    root: String,
    hooks: Arc<Hooks>,
}
pub(crate) fn install(root: &str) -> Installed {
    let value = Arc::new(Hooks::default());
    let old = ROOTS.get_or_init(Default::default).lock().unwrap().insert(root.into(), value.clone());
    assert!(old.is_none(), "one diagnostic hook per actual Root UUID");
    Installed { root: root.into(), hooks: value }
}
impl Installed {
    pub(crate) fn on_elapsed(&self, hook: impl FnOnce(Option<i64>) + Send + 'static) {
        *self.hooks.elapsed.lock().unwrap() = Some(Box::new(hook));
    }
    pub(crate) fn on_gateway(&self, hook: impl FnOnce(&rusqlite::Connection) + Send + 'static) {
        *self.hooks.gateway.lock().unwrap() = Some(Box::new(hook));
    }
    pub(crate) fn observations(&self) -> Vec<Observation> {
        self.hooks.events.lock().unwrap().clone()
    }
}
impl Drop for Installed {
    fn drop(&mut self) {
        if let Some(roots) = ROOTS.get() {
            roots.lock().unwrap().remove(&self.root);
        }
    }
}
pub(crate) fn record(root: &str, branch: &str, detail: &str) {
    let Some(hooks) = hooks(root) else { return };
    let mut events = hooks.events.lock().unwrap();
    if events.len() < 64 {
        events.push(Observation { branch: branch.into(), detail: detail.chars().take(512).collect() });
    }
}
pub(crate) fn observe_error<T, E: std::fmt::Display>(root: &str, branch: &str, result: &Result<T, E>) {
    if let Err(error) = result { record(root, branch, &error.to_string()); }
}
pub(crate) fn after_elapsed(root: &str, elapsed: Option<i64>) {
    // Only pause the original supervisor reader. Foreground accounting keeps
    // its normal transaction and can commit a genuine newer clock sample.
    if std::thread::current().name() != Some("native-worker-supervisor") { return }
    let Some(hooks) = hooks(root) else { return };
    let hook = hooks.elapsed.lock().unwrap().take();
    if let Some(hook) = hook { hook(elapsed); }
}
pub(crate) fn before_gateway(root: &str, db: &rusqlite::Connection) {
    let Some(hooks) = hooks(root) else { return };
    let hook = hooks.gateway.lock().unwrap().take();
    if let Some(hook) = hook { hook(db); }
}
