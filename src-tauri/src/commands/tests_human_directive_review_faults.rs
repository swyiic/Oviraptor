#[test]
fn human_directive_review_stale_scope_or_coordinator_fence_never_changes_business_rows() {
    for fault in [
        "revision",
        "hash",
        "scan",
        "attempt",
        "root_closed",
        "fence",
        "deleted",
    ] {
        let (root, db, draft) = human_review_fixture();
        match fault {
            "attempt" => {
                db.execute(
                    "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
                    [&draft.scan_id],
                )
                .unwrap();
            }
            "root_closed" => {
                db.execute(
                    "UPDATE agent_runs SET status='completed' WHERE id=?1",
                    [&draft.root_run_id],
                )
                .unwrap();
            }
            "fence" => {
                db.execute("UPDATE agent_coordinator_leases SET fencing_token='rotated' WHERE root_run_id=?1",[&draft.root_run_id]).unwrap();
            }
            "deleted" => {
                db.execute(
                    "INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",
                    [&draft.scan_id],
                )
                .unwrap();
            }
            _ => {}
        }
        let before = application_table_snapshot(&db);
        for kind in ["revise", "reject"] {
            assert!(
                review_scan_directive_in(
                    &db,
                    if fault == "scan" {
                        "wrong-scan"
                    } else {
                        &draft.scan_id
                    },
                    &draft.id,
                    if fault == "revision" {
                        draft.revision + 1
                    } else {
                        draft.revision
                    },
                    if fault == "hash" {
                        "wrong-hash"
                    } else {
                        &draft.draft_hash
                    },
                    kind,
                    "有效的新判断"
                )
                .is_err(),
                "{fault}/{kind}"
            );
            assert_eq!(
                application_table_snapshot(&db),
                before,
                "{fault}/{kind} wrote business rows"
            );
        }
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn human_directive_review_silent_writes_or_trigger_damage_roll_back_every_row() {
    let faults=[
        "CREATE TRIGGER damage BEFORE INSERT ON agent_directive_human_reviews BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER damage AFTER INSERT ON agent_directive_human_reviews BEGIN DELETE FROM agent_directive_human_reviews WHERE draft_id=NEW.draft_id; END;",
        "CREATE TRIGGER damage BEFORE UPDATE OF status ON agent_directive_drafts BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER damage AFTER INSERT ON agent_directive_drafts WHEN NEW.revision>1 BEGIN DELETE FROM agent_directive_drafts WHERE id=NEW.id; END;",
        "CREATE TRIGGER damage AFTER INSERT ON agent_directive_human_reviews WHEN NEW.kind='revise' BEGIN UPDATE agent_directive_drafts SET safe_execution_text='silently altered' WHERE id=json_extract(NEW.receipt_json,'$.successorDraftId'); END;",
        "CREATE TRIGGER damage AFTER INSERT ON agent_directive_human_reviews BEGIN UPDATE agent_runs SET plan_json=json_set(plan_json,'$.unexpectedSideEffect',true) WHERE id=NEW.root_run_id; END;",
        "CREATE TRIGGER damage AFTER INSERT ON agent_directive_human_reviews BEGIN INSERT INTO agent_user_directives(id,scan_id,attempt_number,text_redacted,source_draft_id) VALUES('trigger-premature-queue',NEW.scan_id,NEW.attempt_number,'silent premature queue',NEW.draft_id); END;",
        "CREATE TRIGGER damage AFTER INSERT ON agent_directive_human_reviews BEGIN UPDATE agent_directive_drafts SET text_redacted='silently changed original' WHERE id=NEW.draft_id; END;",
        "CREATE TRIGGER damage AFTER INSERT ON agent_directive_human_reviews BEGIN UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1 WHERE root_run_id=NEW.root_run_id; END;",
    ];
    for (index, sql) in faults.iter().enumerate() {
        for kind in ["revise", "reject"] {
            if (index == 3 || index == 4) && kind == "reject" {
                continue;
            }
            let (root, db, draft) = human_review_fixture();
            db.execute_batch(sql).unwrap();
            let before = application_table_snapshot(&db);
            assert!(
                review_scan_directive_in(
                    &db,
                    &draft.scan_id,
                    &draft.id,
                    draft.revision,
                    &draft.draft_hash,
                    kind,
                    "@source_analyst 请关注路径边界"
                )
                .is_err(),
                "fault {index}/{kind} was reported successful"
            );
            assert_eq!(
                application_table_snapshot(&db),
                before,
                "fault {index}/{kind} did not roll back all tables"
            );
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn human_directive_review_approval_receipt_or_queue_damage_rolls_back_confirmation() {
    use crate::agent_runtime::multi_agent::directive;
    for sql in [
        "CREATE TRIGGER damage BEFORE INSERT ON agent_directive_human_reviews BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER damage AFTER INSERT ON agent_directive_human_reviews BEGIN DELETE FROM agent_user_directives WHERE source_draft_id=NEW.draft_id; END;",
        "CREATE TRIGGER damage AFTER INSERT ON agent_directive_human_reviews BEGIN UPDATE agent_user_directives SET payload_json='{}' WHERE source_draft_id=NEW.draft_id; END;",
        "CREATE TRIGGER damage AFTER INSERT ON agent_directive_human_reviews BEGIN UPDATE agent_runs SET plan_json=json_set(plan_json,'$.unexpectedSideEffect',true) WHERE id=NEW.root_run_id; END;",
    ] {
        let (root,db,draft)=human_review_fixture();db.execute_batch(sql).unwrap();
        let before=application_table_snapshot(&db);
        assert!(directive::confirm_bound_draft(&db,&draft.scan_id,&draft.id,draft.revision,&draft.draft_hash).is_err(),"damaged confirmation claimed successful");
        assert_eq!(application_table_snapshot(&db),before,"all queue/draft/receipt/event/Native rows must roll back");
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

include!("tests_human_directive_review_write_guard.rs");

#[test]
fn human_directive_review_corrupt_receipt_is_isolated_but_database_errors_propagate() {
    use crate::agent_runtime::multi_agent::directive;
    for fault in ["malformed_json", "foreign_scope", "missing_table"] {
        let (root, db, draft) = human_review_fixture();
        directive::confirm_bound_draft(
            &db,
            &draft.scan_id,
            &draft.id,
            draft.revision,
            &draft.draft_hash,
        )
        .unwrap();
        db.execute_batch("DROP TRIGGER agent_directive_human_review_immutable")
            .unwrap();
        match fault {
            "malformed_json" => {
                db.pragma_update(None, "ignore_check_constraints", true)
                    .unwrap();
                db.execute(
                    "UPDATE agent_directive_human_reviews SET receipt_json='{' WHERE draft_id=?1",
                    [&draft.id],
                )
                .unwrap();
            }
            "foreign_scope" => {
                db.execute("UPDATE agent_directive_human_reviews SET receipt_json=json_set(receipt_json,'$.rootRunId','foreign') WHERE draft_id=?1", [&draft.id]).unwrap();
            }
            _ => db
                .execute_batch("DROP TABLE agent_directive_human_reviews")
                .unwrap(),
        }
        let before = application_table_snapshot(&db);
        let changes = db.total_changes();
        let result = native_scan_status_after(&db, &draft.scan_id, None);
        if fault == "missing_table" {
            assert!(result.is_err(), "database failure must remain visible");
        } else {
            let status = result.unwrap();
            let item = status["timeline"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == draft.id)
                .unwrap();
            assert!(item["humanReview"].is_null());
            assert_eq!(item["deliveryState"], "receipt_unverified");
            assert!(item["reasonCodes"]
                .as_array()
                .unwrap()
                .contains(&json!("human_review_receipt_unverified")));
        }
        assert_eq!(
            db.total_changes(),
            changes,
            "projection must not repair history"
        );
        assert_eq!(application_table_snapshot(&db), before);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn human_directive_review_damaged_decision_cannot_claim_role_execution_in_timeline() {
    use crate::agent_runtime::multi_agent::directive;
    for damage in [
        "'$.executionCompleted',json('true')",
        "'$.terminal',json('false')",
        "'$.actions[0].executionState','completed'",
        "'$.actions[0].executionReceipt',json('{}')",
        "'$.actions[0].role','foreign_role'",
        "'$.actions[0].order',2",
        "'$.actions[0].intent','foreign_intent'",
        "'$.actions[0].reasonCode','spoofed'",
        "'$.requiresConfirmation',json('true')",
        "'$.successorDraftId','phantom'",
    ] {
        let (root, db, draft) = human_review_fixture();
        directive::confirm_bound_draft(
            &db,
            &draft.scan_id,
            &draft.id,
            draft.revision,
            &draft.draft_hash,
        )
        .unwrap();
        db.execute_batch("DROP TRIGGER agent_directive_human_review_immutable")
            .unwrap();
        db.execute(&format!("UPDATE agent_directive_human_reviews SET receipt_json=json_set(receipt_json,{damage}) WHERE draft_id=?1"), [&draft.id]).unwrap();
        let before = application_table_snapshot(&db);
        let status = native_scan_status_after(&db, &draft.scan_id, None).unwrap();
        let item = status["timeline"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == draft.id)
            .unwrap();
        assert!(item["humanReview"].is_null(), "{damage}: {item}");
        assert_eq!(item["deliveryState"], "receipt_unverified");
        assert_eq!(application_table_snapshot(&db), before);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
