//! Multi-agent runtime contracts that Stage 1A only *persists* (§4, §5, §6).
//!
//! Nothing here schedules, leases or runs an agent: there is no tick, no worker and
//! no second model loop. The assignment row and its state machine exist so a later
//! stage can hand work to a role without inventing storage on the fly.

// Stage 1A declares the contract and its storage only; the scheduler, the child
// runs and the review gate that consume them land in the next stages. Every item
// here is exercised by the Stage 1A tests, so the reachability warning is expected.
#![allow(dead_code)]

pub mod assignment;
