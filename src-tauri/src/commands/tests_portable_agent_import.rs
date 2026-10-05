#[test]
fn portable_agent_import_accepts_only_current_skill_and_knowledge_bundles() {
    let root = std::env::temp_dir().join(format!("oviraptor-portable-agent-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    let source = root.join("portable.json");
    let skills = serde_json::json!({
        "schemaVersion": 1,
        "kind": "oviraptor-agent-skills",
        "skills": [{"name": "Current skill", "instructions": "Read-only analysis", "enabled": true}]
    });
    let knowledge = serde_json::json!({
        "schemaVersion": 1,
        "kind": "oviraptor-agent-knowledge",
        "entries": [{"title": "Current knowledge", "skillInstructions": "Compare evidence",
                     "sourceHash": "portable-source-hash", "patterns": {"knowledgeKind": "method"}}]
    });

    for old_kind in ["oviraptor-strix-skills", "asset-atlas-strix-skills"] {
        let mut old = skills.clone();
        old["kind"] = old_kind.into();
        fs::write(&source, old.to_string()).unwrap();
        assert!(import_agent_instructions_path(&db_path, &source).is_err());
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_skills WHERE name='Current skill'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }
    fs::write(&source, skills.to_string()).unwrap();
    assert_eq!(import_agent_instructions_path(&db_path, &source).unwrap(), 1);
    assert_eq!(connection.query_row("SELECT instructions FROM agent_skills WHERE name='Current skill'", [], |row| row.get::<_, String>(0)).unwrap(), "Read-only analysis");

    for old_kind in ["oviraptor-strix-knowledge", "asset-atlas-strix-knowledge"] {
        let mut old = knowledge.clone();
        old["kind"] = old_kind.into();
        fs::write(&source, old.to_string()).unwrap();
        assert!(import_agent_knowledge_path(&db_path, &source).is_err());
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_knowledge_entries WHERE source_hash='portable-source-hash'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }
    fs::write(&source, knowledge.to_string()).unwrap();
    assert_eq!(import_agent_knowledge_path(&db_path, &source).unwrap(), 1);
    assert_eq!(connection.query_row("SELECT title FROM agent_knowledge_entries WHERE source_hash='portable-source-hash'", [], |row| row.get::<_, String>(0)).unwrap(), "Current knowledge");

    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
