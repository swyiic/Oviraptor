use super::{
    assignment::{insert_assignment, AgentAssignment, AssignmentInsert},
    lease::{
        require_active_attempt, require_executable_coordinator, require_open_coordinator,
        validate_coordinator_lease, CoordinatorLease,
    },
};
use crate::agent_runtime::{
    contract::{AgentBackendKind, AgentLane, AgentRole, AgentRunStatus, MultiAgentPolicy},
    store::{self, AgentRunRow},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledChild {
    pub assignment_id: String,
    pub run_id: String,
    pub role: AgentRole,
}

// Keep the scheduler contracts in one Rust module while grouping each phase
// by business responsibility. The three inclusions share only this boundary.
include!("scheduler/bootstrap.rs");
include!("scheduler/supervised_bootstrap.rs");
include!("scheduler/budget_reservation.rs");
include!("scheduler/schedule_authority.rs");
include!("scheduler/child_contract.rs");
include!("scheduler/child_grant.rs");
include!("scheduler/assignment.rs");
include!("scheduler/reassignment.rs");
include!("scheduler/lifecycle.rs");
include!("scheduler/heartbeat.rs");
