#[test]
fn human_directive_review_write_guard_receipt_cannot_touch_projects_other_root_or_old_draft() {
    use crate::agent_runtime::multi_agent::directive;
    for fault in ["projects", "other_root", "old_draft"] {
        for kind in ["approve", "revise", "reject"] {
            let (root, db, draft) = human_review_fixture();
            db.execute("INSERT INTO agent_runs(id,scan_id,attempt_number,plan_json,status) VALUES('human-review-other-root',?1,1,?2,'completed')",
            params![draft.scan_id,"{\"native\":{\"unchanged\":\"whole-second-Root-JSON\"}}"] ).unwrap();
            let old = draft_scan_directive_in(
                &db,
                &draft.scan_id,
                "@source_analyst 另一条未采纳消息",
                "team",
            )
            .unwrap();
            let old_id = old["id"].as_str().unwrap();
            let action: String = match fault {
                "projects" => "UPDATE projects SET name=name||'-damaged'".into(),
                "other_root" => {
                    "UPDATE agent_runs SET plan_json='{}' WHERE id='human-review-other-root'".into()
                }
                _ => format!(
                    "UPDATE agent_directive_drafts SET status='cancelled' WHERE id='{old_id}'"
                ),
            };
            db.execute_batch(&format!("CREATE TRIGGER hidden_business_write AFTER INSERT ON agent_directive_human_reviews BEGIN {action}; END;")).unwrap();
            let before = application_table_snapshot(&db);
            let result = if kind == "approve" {
                directive::confirm_bound_draft(
                    &db,
                    &draft.scan_id,
                    &draft.id,
                    draft.revision,
                    &draft.draft_hash,
                )
                .map(|_| JsonValue::Null)
            } else {
                review_scan_directive_in(
                    &db,
                    &draft.scan_id,
                    &draft.id,
                    draft.revision,
                    &draft.draft_hash,
                    kind,
                    "@source_analyst 修正边界判断",
                )
            };
            assert!(
                result.is_err(),
                "{fault}/{kind} receipt wrote unrelated business rows"
            );
            assert_eq!(
                application_table_snapshot(&db),
                before,
                "{fault}/{kind} failed full rollback"
            );
            // Private decision write guard must be removed on the error path.
            db.execute_batch("DROP TRIGGER hidden_business_write")
                .unwrap();
            assert!(review_scan_directive_in(
                &db,
                &draft.scan_id,
                &draft.id,
                draft.revision,
                &draft.draft_hash,
                "reject",
                "正常拒绝并保留原始证据"
            )
            .is_ok());
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn human_directive_review_write_guard_receipt_replace_all_unique_keys_cannot_erase_old_decision() {
    for collision in ["receipt_id", "sequence", "draft_revision"] {
        let (root, db, draft) = human_review_fixture();
        review_scan_directive_in(
            &db,
            &draft.scan_id,
            &draft.id,
            draft.revision,
            &draft.draft_hash,
            "reject",
            "第一次终态回执",
        )
        .unwrap();
        let next = draft_scan_directive_in(
            &db,
            &draft.scan_id,
            "@source_analyst 第二条独立草稿",
            "team",
        )
        .unwrap();
        let next_id = next["id"].as_str().unwrap();
        db.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
        let before = application_table_snapshot(&db);
        let sequence = if collision == "sequence" {
            "sequence"
        } else {
            "NULL"
        };
        let receipt_id = if collision == "receipt_id" {
            "receipt_id".into()
        } else {
            format!("'{}'", Uuid::new_v4())
        };
        let chosen_id = if collision == "draft_revision" {
            draft.id.as_str()
        } else {
            next_id
        };
        let sql=format!("INSERT OR REPLACE INTO agent_directive_human_reviews
            (sequence,receipt_id,draft_id,revision,draft_hash,scan_id,attempt_number,root_run_id,target_key,thread_key,kind,argument_hash,receipt_json)
            SELECT {sequence},{receipt_id},?1,revision,draft_hash,scan_id,attempt_number,root_run_id,target_key,thread_key,kind,argument_hash,receipt_json
            FROM agent_directive_human_reviews WHERE draft_id=?2");
        assert!(
            db.execute(&sql, params![chosen_id, draft.id]).is_err(),
            "{collision} silently replaced immutable decision"
        );
        assert_eq!(
            application_table_snapshot(&db),
            before,
            "{collision} implicitly deleted original Native/human records"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn human_directive_review_write_guard_wrong_event_scope_or_replaced_old_event_rolls_back() {
    for changed in ["foreign_scope", "replace_old_event", "omitted_event"] {
        for kind in ["approve", "revise", "reject"] {
            let (root, db, draft) = human_review_fixture();
            // Keep the known emitter name: writer authorizer alone is not an
            // event scope postcondition. SQLite actually runs this altered trigger.
            db.execute_batch(
                "DROP TRIGGER agent_collaboration_draft_update; PRAGMA recursive_triggers=OFF",
            )
            .unwrap();
            let values=match changed {
                "foreign_scope"=>"NEW.scan_id||'-foreign',NEW.attempt_number,'directive_draft',NEW.id,'directive_draft','{}'",
                "replace_old_event"=>"NEW.scan_id,NEW.attempt_number,'directive_draft',NEW.id,'directive_draft','{}'",
                _=>"NEW.scan_id,NEW.attempt_number,'directive_draft',NEW.id,'directive_draft','{}'",
            };
            let body = if changed == "replace_old_event" {
                format!("INSERT OR REPLACE INTO agent_collaboration_events(sequence,scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
                    SELECT MIN(sequence),{values} FROM agent_collaboration_events;")
            } else if changed == "omitted_event" {
                "SELECT 1;".into()
            } else {
                format!("INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) VALUES({values});")
            };
            db.execute_batch(&format!("CREATE TRIGGER agent_collaboration_draft_update AFTER UPDATE OF status ON agent_directive_drafts BEGIN {body} END;")).unwrap();
            let before = application_table_snapshot(&db);
            let result = if kind == "approve" {
                crate::agent_runtime::multi_agent::directive::confirm_bound_draft(
                    &db,
                    &draft.scan_id,
                    &draft.id,
                    draft.revision,
                    &draft.draft_hash,
                )
                .map(|_| JsonValue::Null)
            } else {
                review_scan_directive_in(
                    &db,
                    &draft.scan_id,
                    &draft.id,
                    draft.revision,
                    &draft.draft_hash,
                    kind,
                    "@source_analyst 确认新的边界描述",
                )
            };
            assert!(
                result.is_err(),
                "{changed}/{kind} claimed scoped durable events"
            );
            assert_eq!(
                application_table_snapshot(&db),
                before,
                "{changed}/{kind} corrupted event replay/history"
            );
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
}
