//! Credential→keychain migration, Phase 0 (see docs/KEYCHAIN_MIGRATION_PLAN.md).
//!
//! This builds the secret-store abstraction and a macOS keychain backend. It is
//! deliberately INERT for the running app: nothing here is called from live
//! credential resolution yet, and read-through is OFF unless `OVIRAPTOR_KEYCHAIN`
//! is set — and even then `resolve`/`store` fall back to the caller's existing
//! plaintext, so behaviour cannot regress. Later phases wire it into
//! `model_configuration` / worker credential paths and add Windows/Linux backends.
//!
//! The whole module is `allow(dead_code)` on purpose: these are forward-scaffolding
//! items the next phase consumes, not orphans.

use std::sync::atomic::{compiler_fence, AtomicBool, Ordering};

/// Environment override for the read-through toggle. Off unless set to "1"/"true"
/// (case-insensitive). The normal control is the user-facing setting below; this
/// stays as an override for tests and headless runs.
pub const KEYCHAIN_ENV: &str = "OVIRAPTOR_KEYCHAIN";

/// Fixed service name for every Oviraptor keychain item.
pub const KEYCHAIN_SERVICE: &str = "com.oviraptor.desktop";

/// Runtime mirror of the persisted user setting (app_settings `keychain_enabled`),
/// loaded once at startup and updated when the user toggles it in the config
/// center. Kept in an atomic so resolution needs no DB access on its path.
static KEYCHAIN_SETTING: AtomicBool = AtomicBool::new(false);

/// Set by startup (from the persisted setting) and by the toggle command.
pub fn set_keychain_setting(enabled: bool) {
    KEYCHAIN_SETTING.store(enabled, Ordering::SeqCst);
}

/// The current user setting. Resolution turns the keychain on when this OR the
/// environment override is true.
pub fn keychain_setting_enabled() -> bool {
    KEYCHAIN_SETTING.load(Ordering::SeqCst)
}

pub fn keychain_read_through_enabled(env_value: Option<&str>) -> bool {
    matches!(
        env_value.map(|value| value.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true")
    )
}

/// A secret value that wipes its bytes on drop — the same volatile-write hygiene
/// as `artifact_import::sealing::Key`, with no new dependency. Best effort: a
/// `Vec` that reallocated leaves the old allocation untouched, but a buffer that
/// did not reallocate is overwritten before it is freed.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn from_str(value: &str) -> Self {
        Self(value.as_bytes().to_vec())
    }
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        for byte in self.0.iter_mut() {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        compiler_fence(Ordering::SeqCst);
    }
}

/// Stable logical identity for one secret. Scoped per profile so multiple model
/// profiles never overwrite each other's key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretId {
    kind: &'static str,
    scope: String,
}

impl SecretId {
    /// The keychain account name: `kind@scope`, or just `kind` when unscoped.
    pub fn account(&self) -> String {
        if self.scope.is_empty() {
            self.kind.to_string()
        } else {
            format!("{}@{}", self.kind, self.scope)
        }
    }
}

pub fn llm_api_key(profile: &str) -> SecretId {
    SecretId {
        kind: "llm.apiKey",
        scope: profile.to_string(),
    }
}
pub fn llm_local_api_key(profile: &str) -> SecretId {
    SecretId {
        kind: "llm.localApiKey",
        scope: profile.to_string(),
    }
}
pub fn oauth_token(profile: &str) -> SecretId {
    SecretId {
        kind: "oauth.token",
        scope: profile.to_string(),
    }
}
pub fn worker_access_token() -> SecretId {
    SecretId {
        kind: "worker.accessToken",
        scope: String::new(),
    }
}

/// A place secrets can live. Backends never panic. A missing secret is `Ok(None)`;
/// an operational failure is `Err`, which callers treat as "unavailable, fall
/// back" — never as "there is no secret".
pub trait SecretBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn available(&self) -> bool;
    fn get(&self, id: &SecretId) -> Result<Option<Secret>, String>;
    fn set(&self, id: &SecretId, secret: &Secret) -> Result<(), String>;
    fn delete(&self, id: &SecretId) -> Result<(), String>;
}

#[cfg(target_os = "macos")]
pub struct KeychainBackend {
    service: String,
}

#[cfg(target_os = "macos")]
impl KeychainBackend {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
    fn entry(&self, id: &SecretId) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.service, &id.account()).map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "macos")]
impl SecretBackend for KeychainBackend {
    fn name(&self) -> &'static str {
        "keychain"
    }
    fn available(&self) -> bool {
        // Non-prompting: constructing an entry touches no stored item. A real
        // read/write probe is deferred to the phase that turns read-through on,
        // because a probe would trigger the macOS access dialog.
        keyring::Entry::new(&self.service, "__availability__").is_ok()
    }
    fn get(&self, id: &SecretId) -> Result<Option<Secret>, String> {
        match self.entry(id)?.get_password() {
            Ok(value) => Ok(Some(Secret::from_str(&value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
    fn set(&self, id: &SecretId, secret: &Secret) -> Result<(), String> {
        let value = secret
            .as_str()
            .ok_or_else(|| "密钥不是有效 UTF-8".to_string())?;
        self.entry(id)?
            .set_password(value)
            .map_err(|error| error.to_string())
    }
    fn delete(&self, id: &SecretId) -> Result<(), String> {
        match self.entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Composes an optional keychain backend with the caller's existing plaintext
/// source. With read-through OFF it is a pure pass-through, so wiring it into a
/// credential path changes nothing until a later phase turns it on.
pub struct SecretStore {
    keychain: Option<Box<dyn SecretBackend>>,
    read_through: bool,
}

impl SecretStore {
    pub fn new(keychain: Option<Box<dyn SecretBackend>>, read_through: bool) -> Self {
        Self {
            keychain,
            read_through,
        }
    }

    /// The store the running app uses: real keychain on macOS, read-through taken
    /// from the environment (OFF by default).
    pub fn system() -> Self {
        let read_through = keychain_setting_enabled()
            || keychain_read_through_enabled(std::env::var(KEYCHAIN_ENV).ok().as_deref());
        #[cfg(target_os = "macos")]
        let keychain: Option<Box<dyn SecretBackend>> =
            Some(Box::new(KeychainBackend::new(KEYCHAIN_SERVICE)));
        #[cfg(not(target_os = "macos"))]
        let keychain: Option<Box<dyn SecretBackend>> = None;
        Self::new(keychain, read_through)
    }

    /// The keychain backend to use right now, or `None` when read-through is off or
    /// the backend is unavailable (both mean "use the caller's plaintext").
    fn active(&self) -> Option<&dyn SecretBackend> {
        if !self.read_through {
            return None;
        }
        self.keychain
            .as_deref()
            .filter(|backend| backend.available())
    }

    /// Resolve a secret. `legacy` is the current plaintext from DB/file. OFF or
    /// keychain-unavailable → `legacy` unchanged. ON with `legacy` present → the
    /// plaintext stays authoritative (the P1–P3 double-store) and the keychain is
    /// synced to it, so a key rotated in the DB propagates on the next read and a
    /// later phase can blank the plaintext and read from the keychain. ON with no
    /// plaintext left → the keychain value. A sync failure never fails resolution,
    /// so the caller is never locked out of a working credential.
    pub fn resolve(&self, id: &SecretId, legacy: Option<&str>) -> Option<String> {
        let normalized = legacy.map(str::trim).filter(|value| !value.is_empty());
        let Some(backend) = self.active() else {
            return normalized.map(str::to_string);
        };
        if let Some(value) = normalized {
            let _ = backend.set(id, &Secret::from_str(value));
            return Some(value.to_string());
        }
        if let Ok(Some(found)) = backend.get(id) {
            if let Some(value) = found.as_str().map(str::trim).filter(|v| !v.is_empty()) {
                return Some(value.to_string());
            }
        }
        None
    }

    /// Record a new/updated secret. OFF or keychain-unavailable → no-op: the caller
    /// keeps persisting plaintext exactly as today (the double-store is deliberate,
    /// so a later keychain loss cannot strand the credential).
    pub fn store(&self, id: &SecretId, value: &str) -> Result<(), String> {
        match self.active() {
            Some(backend) => backend.set(id, &Secret::from_str(value)),
            None => Ok(()),
        }
    }
}

/// In-memory keychain stand-in, shared across the crate's test builds so other
/// modules (e.g. model_configuration) can inject a store and inspect it.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    pub(crate) struct MemoryBackend {
        up: bool,
        map: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    }
    impl MemoryBackend {
        pub(crate) fn new(up: bool) -> Self {
            Self {
                up,
                map: Arc::new(Mutex::new(HashMap::new())),
            }
        }
        pub(crate) fn len(&self) -> usize {
            self.map.lock().unwrap().len()
        }
    }
    impl SecretBackend for MemoryBackend {
        fn name(&self) -> &'static str {
            "memory"
        }
        fn available(&self) -> bool {
            self.up
        }
        fn get(&self, id: &SecretId) -> Result<Option<Secret>, String> {
            Ok(self
                .map
                .lock()
                .unwrap()
                .get(&id.account())
                .map(|bytes| Secret::from_bytes(bytes.clone())))
        }
        fn set(&self, id: &SecretId, secret: &Secret) -> Result<(), String> {
            self.map
                .lock()
                .unwrap()
                .insert(id.account(), secret.expose().to_vec());
            Ok(())
        }
        fn delete(&self, id: &SecretId) -> Result<(), String> {
            self.map.lock().unwrap().remove(&id.account());
            Ok(())
        }
    }

    /// A store backed by an in-memory keychain, plus a handle to inspect it after
    /// it has been moved into the store.
    pub(crate) fn store(up: bool, read_through: bool) -> (SecretStore, MemoryBackend) {
        let backend = MemoryBackend::new(up);
        (
            SecretStore::new(Some(Box::new(backend.clone())), read_through),
            backend,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testing::MemoryBackend;
    use super::*;

    #[test]
    fn flag_off_is_pure_passthrough_and_never_touches_the_backend() {
        let backend = MemoryBackend::new(true);
        let store = SecretStore::new(Some(Box::new(backend.clone())), false);
        assert_eq!(
            store.resolve(&llm_api_key("p1"), Some("legacy")),
            Some("legacy".to_string())
        );
        store.store(&llm_api_key("p1"), "new").unwrap();
        assert_eq!(store.resolve(&llm_api_key("p1"), None), None);
        assert_eq!(backend.len(), 0, "read-through OFF must not write the keychain");
    }

    #[test]
    fn on_keeps_db_authoritative_and_syncs_the_keychain() {
        let backend = MemoryBackend::new(true);
        let store = SecretStore::new(Some(Box::new(backend.clone())), true);
        // Plaintext present: returns it and syncs it into the keychain.
        assert_eq!(
            store.resolve(&llm_api_key("p1"), Some("legacy")),
            Some("legacy".to_string())
        );
        assert_eq!(backend.len(), 1);
        // A key rotated in the DB wins over the now-stale keychain copy, and the
        // keychain is re-synced to the new value.
        assert_eq!(
            store.resolve(&llm_api_key("p1"), Some("rotated")),
            Some("rotated".to_string())
        );
        // With the plaintext gone (a later phase blanked it), the keychain — now
        // holding the rotated value — becomes the source of truth.
        assert_eq!(
            store.resolve(&llm_api_key("p1"), None),
            Some("rotated".to_string())
        );
    }

    #[test]
    fn unavailable_keychain_falls_back_and_is_never_written() {
        let backend = MemoryBackend::new(false);
        let store = SecretStore::new(Some(Box::new(backend.clone())), true);
        assert_eq!(
            store.resolve(&llm_api_key("p1"), Some("legacy")),
            Some("legacy".to_string())
        );
        store.store(&llm_api_key("p1"), "new").unwrap();
        assert_eq!(backend.len(), 0);
    }

    #[test]
    fn store_then_resolve_round_trips_when_on() {
        let backend = MemoryBackend::new(true);
        let store = SecretStore::new(Some(Box::new(backend)), true);
        store.store(&oauth_token("p1"), "tok").unwrap();
        assert_eq!(
            store.resolve(&oauth_token("p1"), None),
            Some("tok".to_string())
        );
    }

    #[test]
    fn empty_legacy_and_blank_keychain_resolve_to_nothing() {
        let backend = MemoryBackend::new(true);
        let store = SecretStore::new(Some(Box::new(backend)), true);
        assert_eq!(store.resolve(&llm_api_key("p1"), Some("   ")), None);
        assert_eq!(store.resolve(&llm_api_key("p1"), None), None);
    }

    #[test]
    fn per_profile_ids_do_not_collide() {
        assert_eq!(llm_api_key("p1").account(), "llm.apiKey@p1");
        assert_ne!(llm_api_key("p1").account(), llm_api_key("p2").account());
        assert_eq!(worker_access_token().account(), "worker.accessToken");
    }

    #[test]
    fn read_through_flag_parsing() {
        assert!(keychain_read_through_enabled(Some("1")));
        assert!(keychain_read_through_enabled(Some("true")));
        assert!(keychain_read_through_enabled(Some("TRUE")));
        assert!(!keychain_read_through_enabled(Some("0")));
        assert!(!keychain_read_through_enabled(Some("yes")));
        assert!(!keychain_read_through_enabled(None));
    }

    #[test]
    fn the_persisted_setting_flag_round_trips() {
        set_keychain_setting(true);
        assert!(keychain_setting_enabled());
        set_keychain_setting(false);
        assert!(!keychain_setting_enabled());
    }

    // Exercises the REAL macOS login keychain (the piece the in-memory tests above
    // cannot cover). #[ignore] so it never runs in CI or on a machine without a
    // keychain; run explicitly with `cargo test --lib --ignored keychain_self_test`.
    // Uses a dedicated test service and a clearly-synthetic DeepSeek-shaped key, and
    // deletes what it writes, so it never touches a real credential or the app's
    // real "com.oviraptor.desktop" items.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches the real macOS login keychain; run with --ignored"]
    fn keychain_self_test_round_trips_a_synthetic_deepseek_key() {
        const TEST_SERVICE: &str = "com.oviraptor.desktop.selftest";
        const SYNTHETIC: &str = "sk-deepseek-SYNTHETIC-donotuse-0123456789";
        let backend = KeychainBackend::new(TEST_SERVICE);
        assert!(backend.available(), "keychain backend reports unavailable");
        let id = llm_api_key("deepseek-selftest");
        let _ = backend.delete(&id); // start clean even if a prior run was interrupted

        let store = SecretStore::new(Some(Box::new(KeychainBackend::new(TEST_SERVICE))), true);
        // Plaintext present → returned as-is and synced into the real keychain.
        assert_eq!(
            store.resolve(&id, Some(SYNTHETIC)),
            Some(SYNTHETIC.to_string())
        );
        // Plaintext gone → the value is served back out of the real keychain.
        assert_eq!(store.resolve(&id, None), Some(SYNTHETIC.to_string()));
        // And a direct backend read matches what the store wrote.
        let direct = backend
            .get(&id)
            .unwrap()
            .and_then(|secret| secret.as_str().map(str::to_string));
        assert_eq!(direct, Some(SYNTHETIC.to_string()));

        backend.delete(&id).unwrap();
        assert!(backend.get(&id).unwrap().is_none(), "cleanup left an item behind");
    }
}
