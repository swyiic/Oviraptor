#[test]
fn budget_web_model_claim_faults_never_send_and_restore_authority() {
    for trigger in [
        "CREATE TRIGGER reject_web_claim BEFORE INSERT ON agent_web_model_journal WHEN NEW.phase='dispatch' BEGIN SELECT RAISE(ABORT,'claim unavailable'); END;",
        "CREATE TRIGGER reject_web_claim BEFORE INSERT ON agent_web_model_journal WHEN NEW.phase='dispatch' BEGIN SELECT RAISE(FAIL,'claim unavailable'); END;",
        "CREATE TRIGGER reject_web_claim BEFORE INSERT ON agent_web_model_journal WHEN NEW.phase='dispatch' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER reject_web_claim AFTER INSERT ON agent_web_model_journal WHEN NEW.phase='dispatch' BEGIN UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=NEW.root_run_id; END;",
    ] {
        let f=web_financial_producer_fixture();let db=db::open(&f.context().db_path).unwrap();db.execute_batch(trigger).unwrap();
        use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
        let fees=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap();
        let outcome=NativeAgentBackend.execute(f.context());assert!(web_inflight_original_path(f.context()).is_file(),"original claim boundary required: {}",outcome.detail());assert!(outcome.completion().is_none());f.assert_calls(0);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_web_model_journal",[],|r|r.get::<_,i64>(0)).unwrap(),0);assert!(Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap()==fees);assert_eq!(db.query_row("SELECT count(*) FROM agent_runs WHERE cancel_requested_at<>''",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
}

#[test]
fn budget_web_model_failed_receipt_reopen_never_replays_or_refunds() {
    use crate::agent_runtime::multi_agent::budget;
    for action in [
        "ABORT,'receipt unavailable'",
        "FAIL,'receipt unavailable'",
        "IGNORE",
    ] {
        let f = web_financial_producer_fixture();
        let mut context = f.context().clone();
        let db = db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER reject_web_receipt BEFORE INSERT ON agent_web_model_journal WHEN NEW.phase='received' BEGIN SELECT RAISE({action}); END;")).unwrap();
        assert!(NativeAgentBackend.execute(&context).completion().is_none());
        f.assert_calls(1);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_web_model_journal WHERE phase='dispatch'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_web_model_journal WHERE phase<>'dispatch'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("DROP TRIGGER reject_web_receipt").unwrap();
        context.resume = true;
        assert_eq!(
            NativeAgentBackend.execute(&context).terminal_code(),
            terminal_code::REQUEST_RECONCILIATION_REQUIRED
        );
        f.assert_calls(1);
        let tx = db.unchecked_transaction().unwrap();
        let (lease, assignment) =
            budget::target::child_owner(&tx, &context.run.as_ref().unwrap().run_id)
                .unwrap()
                .unwrap();
        let before =
            budget::balance(&tx, &lease.root_run_id, Some(&assignment), "model_requests").unwrap();
        assert_eq!(
            (before.reserved, before.consumed, before.indeterminate),
            (1, 0, 0)
        );
        assert_eq!(
            budget::model::release_unsent(&tx, &lease, &assignment).unwrap_err(),
            "budget_indeterminate_requires_reconciliation"
        );
        assert_eq!(
            budget::model::settle(&tx, &lease, &assignment, &Default::default()).unwrap_err(),
            "budget_indeterminate_requires_reconciliation"
        );
        assert_eq!(
            budget::admission::require_settled_for_completion(&tx, &lease.root_run_id).unwrap_err(),
            "budget_indeterminate_requires_reconciliation"
        );
        assert_eq!(
            budget::balance(&tx, &lease.root_run_id, Some(&assignment), "model_requests").unwrap(),
            before
        );
        assert!(tx
            .execute(
                "UPDATE agent_web_model_journal SET request_hash='tampered'",
                []
            )
            .is_err());
        assert!(tx
            .execute("DELETE FROM agent_web_model_journal", [])
            .is_err());
        tx.rollback().unwrap();
    }
}
