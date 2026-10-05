#[test]
fn root_v2_slot_replace_cannot_rebind_an_existing_root_or_declaration_uuid() {
    for conflict in ["root", "declaration"] {
        let h = root_v2_slot_harness(&format!("v2-no-replace-{conflict}"), 1);
        let root = h.context.run.as_ref().unwrap().run_id.clone();
        let db = db::open(&h.db_path).unwrap();
        db.pragma_update(None, "recursive_triggers", false).unwrap();
        let (id,payload,hash): (String,String,String)=db.query_row(
            "SELECT declaration_id,definition_json,native_plan_text_hash FROM agent_root_budget_definitions WHERE root_run_id=?1",
            [&root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let replacement_root = if conflict == "root" {
            root.clone()
        } else {
            "forged-other-root".into()
        };
        let replacement_id = if conflict == "declaration" {
            id
        } else {
            Uuid::new_v4().to_string()
        };
        assert!(db.execute(
            "INSERT OR REPLACE INTO agent_root_budget_definitions(root_run_id,declaration_id,definition_json,native_plan_text_hash) VALUES(?1,?2,?3,?4)",
            params![replacement_root,replacement_id,payload,hash]).is_err(),
            "immutable statement must reject both unique-conflict replacement paths with recursive triggers off");
        assert!(super::tests::application_table_snapshot(&db) == before);
        assert_eq!(h.model_seen.lock().unwrap().len(), 0);
        drop(db);
        fs::remove_dir_all(h.root).unwrap();
    }
}

#[test]
fn root_v2_slot_new_root_insert_cannot_mutate_business_or_prior_root_or_forge_event() {
    for fault in ["business", "prior_root", "projection", "event"] {
        let h = agent_harness(
            &format!("v2-creation-{fault}"),
            mock_site,
            vec![AgentIdentity::anonymous()],
        );
        seed_attempt_row(&h.db_path, h.context.attempt_number, "initial");
        let declaration = root_v2_slot_declaration(&h.context, 1);
        let db = db::open(&h.db_path).unwrap();
        db.execute("INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,plan_hash,plan_json)
            VALUES('prior-v2-root','agent-scan',1,'https://prior.example.test','native','coordinator','old-native-plan','{}')",[]).unwrap();
        let trigger=match fault {
            "business"=>"CREATE TRIGGER root_v2_creation_fault AFTER INSERT ON agent_runs BEGIN INSERT INTO projects(name) VALUES('new-root-unapproved'); END;",
            "prior_root"=>"CREATE TRIGGER root_v2_creation_fault AFTER INSERT ON agent_runs BEGIN UPDATE agent_runs SET used_tokens=17 WHERE id='prior-v2-root'; END;",
            "projection"=>"CREATE TRIGGER root_v2_creation_fault AFTER INSERT ON sentinel_checkpoints WHEN NEW.stage='agent_execution_plan' BEGIN UPDATE agent_runs SET used_tokens=17 WHERE id='prior-v2-root'; END;",
            "event"=>"DROP TRIGGER agent_collaboration_run_insert; CREATE TRIGGER agent_collaboration_run_insert AFTER INSERT ON agent_runs BEGIN INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) VALUES(NEW.scan_id,NEW.attempt_number,'agent_run','prior-v2-root','agent_run','{\"role\":\"coordinator\",\"status\":\"prepared\",\"terminalState\":\"\"}'); END;",
            _=>unreachable!(),
        };
        db.execute_batch(trigger).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let root_before = root_v2_slot_root_row(&db, "prior-v2-root");
        assert!(
            persist_agent_execution_plan_with_budget(
                &h.db_path,
                &h.context.scan_id,
                h.context.attempt_number,
                &h.context.target_url,
                &h.context.execution_plan,
                Some(&declaration)
            )
            .is_err(),
            "{fault}: a sidecar cannot legitimize a collateral Root creation transaction"
        );
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{fault}: every insert/projection/event must roll back"
        );
        assert_eq!(root_v2_slot_root_row(&db, "prior-v2-root"), root_before);
        assert_eq!(h.model_seen.lock().unwrap().len(), 0);
        drop(db);
        fs::remove_dir_all(h.root).unwrap();
    }
}
