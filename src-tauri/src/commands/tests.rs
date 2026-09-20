#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Arc, Mutex};

    // §12: one file per contract area, included into this module so the helpers
    // stay shared without a second copy of any fixture.

    include!("tests_investigation.rs");
    include!("tests_frontend_recon.rs");
    include!("tests_scan_lifecycle.rs");
    include!("tests_results.rs");
    include!("tests_runtime.rs");
    include!("tests_code_rules.rs");
    include!("tests_database_repair.rs");
    include!("tests_model_config.rs");
}
