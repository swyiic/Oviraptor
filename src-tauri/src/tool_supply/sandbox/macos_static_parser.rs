//! Seatbelt platform evidence, not an approved-package production runner.
//! A successful fixture proves only the guarantees it actually exercises.
//! Hard per-run memory containment remains unverified; no execution API is exposed.

pub(crate) struct MacosStaticParserReadiness {
    mechanism_available: bool,
}
impl MacosStaticParserReadiness {
    pub(crate) fn inspect() -> Self {
        #[cfg(target_os = "macos")]
        let available = {
            use std::os::unix::fs::MetadataExt;
            std::fs::symlink_metadata("/usr/bin/sandbox-exec")
                .is_ok_and(|m| m.is_file() && m.uid() == 0 && m.mode() & 0o022 == 0)
        };
        #[cfg(not(target_os = "macos"))]
        let available = false;
        Self {
            mechanism_available: available,
        }
    }
    pub(crate) fn require_supported(&self) -> Result<(), &'static str> {
        if !self.mechanism_available {
            return Err("unsupported_sandbox:macos_seatbelt_unavailable");
        }
        // RLIMIT_CPU is CPU time, not a core quota. RLIMIT_RSS/AS must not
        // silently stand in for an unproved hard per-run memory boundary.
        Err("unsupported_sandbox:macos_hard_memory_containment_unverified")
    }
}

// Keep the fixture's destructor portable so its failure path can be tested even
// when the actual Seatbelt probes cannot run on this platform.


