//! The shared evidence graph: typed contract plus its repository.
//!
//! Stage 1A only stores facts, inferences and relations. Promotion to a finding is a
//! review decision and is never implied by an edge (§9.1).

// Stage 1A declares the contract and its storage only; the scheduler, the child
// runs and the review gate that consume them land in the next stages. Every item
// here is exercised by the Stage 1A tests, so the reachability warning is expected.
#![allow(dead_code)]

pub mod contract;
pub mod store;
