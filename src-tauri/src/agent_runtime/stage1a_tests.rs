// Stage 1A acceptance tests (§7). Keep each concern small while sharing the
// temporary SQLite helpers and existing test scope from tests.rs.
include!("stage1a_tests/contract_assignment.rs");
include!("stage1a_tests/contract_owner.rs");
include!("stage1a_tests/assignment_persistence.rs");
include!("stage1a_tests/evidence_review.rs");
include!("stage1a_tests/evidence_revisions.rs");
include!("stage1a_tests/persistence_integrity.rs");
include!("stage1a_tests/migration.rs");
