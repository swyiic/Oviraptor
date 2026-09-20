#[cfg(test)]
mod agent_tests {
    use super::*;

    // ------------------------------------------------------------------
    // Fixtures
    // ------------------------------------------------------------------

    // §12: the shared fixtures are included first, then one file per contract
    // area. These are textual includes inside this module, so no fixture had to be
    // copied or made public to a sibling test module.

    include!("agent_tests_fixtures.rs");
    include!("agent_tests_policy.rs");
    include!("agent_tests_harness.rs");
    include!("agent_tests_e2e.rs");
    include!("agent_tests_contracts.rs");
    include!("agent_tests_coverage.rs");
    include!("agent_tests_browser.rs");
    include!("agent_tests_validation.rs");
    include!("agent_tests_persistence.rs");
    include!("agent_tests_identity_e2e.rs");
}
