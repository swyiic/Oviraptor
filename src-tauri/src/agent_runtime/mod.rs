//! Oviraptor-owned agent runtime.
//!
//! The orchestrator, budget, identity handles, coverage and terminal state live
//! here. A backend (native loop or Strix) may only report facts; it can never
//! write the terminal state itself.

pub mod checkpoint;
pub mod contract;
pub mod evidence_graph;
pub mod model;
pub mod multi_agent;
pub mod reducer;
pub mod review;
pub mod secrets;
pub mod store;
pub mod strix_adapter;

#[cfg(test)]
mod tests;
