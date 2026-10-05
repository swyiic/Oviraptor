// Stage 1A / §5.3 — one owner per contract across all child runs of a root.
// No website, no model call, no target request is involved.
use super::multi_agent::contract_owner::{
    acquire_contract_owner, contract_key, release_contract_owner,
};

/// Same owner re-acquiring is idempotent; a different assignment must never
/// steal the contract; a stale fencing epoch must fail closed.
#[test]
fn contract_owner_is_single_and_stale_fencing_fails_closed() {
    let (_dir, connection) = stage1a_connection("contract-owner");
    create_runs(&connection, &["root-1", "child-a", "child-b"]);
    let key = contract_key(&["2", "https://app.example.invalid", "GET", "/api/orders"]);
    let first = acquire_contract_owner(&connection, "root-1", &key, "asg-a", 3, "fence-3").unwrap();
    assert!(first.created);
    assert_eq!(first.owner_assignment_id, "asg-a");
    // Idempotent re-acquire by the same owner with the same fencing.
    let again = acquire_contract_owner(&connection, "root-1", &key, "asg-a", 3, "fence-3").unwrap();
    assert!(!again.created);
    // A different assignment must never steal it.
    let err = acquire_contract_owner(&connection, "root-1", &key, "asg-b", 3, "fence-3")
        .expect_err("second owner must be rejected");
    assert_eq!(err, "contract_owned_by_other");
    // The old coordinator (stale epoch) must fail even for the owning assignment.
    let err = acquire_contract_owner(&connection, "root-1", &key, "asg-a", 2, "fence-2")
        .expect_err("stale fencing must fail closed");
    assert_eq!(err, "stale_contract_fencing");
    // Release by a non-owner fails; release by the owner frees the contract.
    let err = release_contract_owner(&connection, "root-1", &key, "asg-b", 3, "fence-3")
        .expect_err("non-owner release must fail");
    assert_eq!(err, "contract_owned_by_other");
    release_contract_owner(&connection, "root-1", &key, "asg-a", 3, "fence-3").unwrap();
    let second =
        acquire_contract_owner(&connection, "root-1", &key, "asg-b", 3, "fence-3").unwrap();
    assert!(second.created);
    assert_eq!(second.owner_assignment_id, "asg-b");
}

/// Owners never leak across roots.
#[test]
fn contract_owners_are_isolated_per_root() {
    let (_dir, connection) = stage1a_connection("contract-owner-roots");
    create_runs(&connection, &["root-1", "root-2"]);
    let key = contract_key(&["2", "https://app.example.invalid", "GET", "/x"]);
    acquire_contract_owner(&connection, "root-1", &key, "asg-a", 1, "fence-1").unwrap();
    let other = acquire_contract_owner(&connection, "root-2", &key, "asg-b", 1, "fence-1").unwrap();
    assert!(other.created);
}
