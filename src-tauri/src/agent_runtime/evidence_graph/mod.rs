//! The shared evidence graph: typed contract plus its repository.
//!
//! Facts, inferences and relations remain distinct. Promotion to a finding is a
//! live reviewer decision and is never implied by an edge (§9.1).

// The live scheduler, child runs and review gate consume this module. Some
// repository APIs remain acceptance-test-only, so the reachability warning is expected.
#![allow(dead_code)]

pub mod contract;
pub mod store;
