fn evidence_node(
    id: &str,
    root_run_id: &str,
    revision: i64,
    kind: super::evidence_graph::contract::EvidenceNodeKind,
    provenance: super::evidence_graph::contract::EvidenceProvenance,
    payload: serde_json::Value,
) -> super::evidence_graph::contract::EvidenceNode {
    super::evidence_graph::contract::EvidenceNode {
        id: id.into(),
        root_run_id: root_run_id.into(),
        revision,
        kind,
        provenance,
        natural_key_hash: super::evidence_graph::store::evidence_natural_key(root_run_id, kind, id),
        payload,
        artifact_refs: vec![],
        created_by_run_id: root_run_id.into(),
        supersedes_id: String::new(),
        created_at: String::new(),
    }
}

fn evidence_edge(
    root_run_id: &str,
    revision: i64,
    from: &str,
    to: &str,
    payload: serde_json::Value,
) -> super::evidence_graph::contract::EvidenceEdge {
    super::evidence_graph::contract::EvidenceEdge {
        id: 0,
        root_run_id: root_run_id.into(),
        revision,
        from_node_id: from.into(),
        to_node_id: to.into(),
        kind: super::evidence_graph::contract::EvidenceEdgeKind::Supports,
        payload,
        created_by_run_id: root_run_id.into(),
        created_at: String::new(),
    }
}

/// §4.4 — the same natural key with other bytes is a conflict, and the old fact
/// stays untouched.
#[test]
fn evidence_node_replay_compares_content() {
    use super::evidence_graph::contract::{EvidenceNodeKind, EvidenceProvenance};
    use super::evidence_graph::store::{insert_evidence_node, load_evidence_node, EvidenceInsert};
    let (root, connection) = stage1a_connection("stage1a-node-conflict");
    create_runs(&connection, &["root-a"]);
    let node = evidence_node(
        "n-1",
        "root-a",
        1,
        EvidenceNodeKind::Endpoint,
        EvidenceProvenance::Observed,
        json!({"method": "GET"}),
    );
    assert_eq!(
        insert_evidence_node(&connection, &node).unwrap(),
        EvidenceInsert::Inserted("n-1".into())
    );
    let drift = super::evidence_graph::contract::EvidenceNode {
        provenance: EvidenceProvenance::Inferred,
        ..node.clone()
    };
    let error = insert_evidence_node(&connection, &drift).unwrap_err();
    assert!(error.contains("evidence_node_conflict"), "{error}");
    let stored = load_evidence_node(&connection, "n-1").unwrap().unwrap();
    assert_eq!(stored.provenance, EvidenceProvenance::Observed);
    let foreign_key = super::evidence_graph::contract::EvidenceNode {
        id: "n-1".into(),
        natural_key_hash: "another-hash".into(),
        ..node.clone()
    };
    let error = insert_evidence_node(&connection, &foreign_key).unwrap_err();
    assert!(error.contains("另一个自然键"), "{error}");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §5.5 — an edge may join earlier facts in the same root, but never future
/// facts or another root. The returned id is the stored rowid.
#[test]
fn evidence_edges_stay_inside_one_root_and_cannot_reference_future_revision() {
    use super::evidence_graph::contract::{EvidenceNodeKind, EvidenceProvenance};
    use super::evidence_graph::store::{
        insert_evidence_edge, insert_evidence_node, list_evidence_edges_at_revision,
        EvidenceInsert,
    };
    let (root, connection) = stage1a_connection("stage1a-edge");
    create_runs(&connection, &["root-a", "root-b"]);
    for (id, root_run, revision) in [
        ("n-a1", "root-a", 1),
        ("n-a2", "root-a", 1),
        ("n-b1", "root-b", 1),
        ("n-a3", "root-a", 2),
    ] {
        insert_evidence_node(
            &connection,
            &evidence_node(
                id,
                root_run,
                revision,
                EvidenceNodeKind::Endpoint,
                EvidenceProvenance::Observed,
                json!({"id": id}),
            ),
        )
        .unwrap();
    }
    let first = insert_evidence_edge(
        &connection,
        &evidence_edge("root-a", 1, "n-a1", "n-a2", json!({"why": "same"})),
    )
    .unwrap();
    let stored_id = match &first {
        EvidenceInsert::Inserted(id) => id.clone(),
        other => panic!("首次插入应返回 Inserted：{other:?}"),
    };
    assert!(
        stored_id.parse::<i64>().unwrap() > 0,
        "首次插入必须返回真实 rowid，而不是调用者的 0：{stored_id}"
    );
    let replay = insert_evidence_edge(
        &connection,
        &super::evidence_graph::contract::EvidenceEdge {
            id: 4242,
            ..evidence_edge("root-a", 1, "n-a1", "n-a2", json!({"why": "same"}))
        },
    )
    .unwrap();
    assert_eq!(replay, EvidenceInsert::Existing(stored_id.clone()));

    let count = |connection: &Connection| -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM agent_evidence_edges", [], |row| row.get(0))
            .unwrap()
    };
    let before = count(&connection);
    let cross_root = insert_evidence_edge(
        &connection,
        &evidence_edge("root-a", 1, "n-a1", "n-b1", json!({})),
    )
    .unwrap_err();
    assert!(cross_root.contains("不属于本 root"), "{cross_root}");
    let cross_revision = insert_evidence_edge(
        &connection,
        &evidence_edge("root-a", 1, "n-a1", "n-a3", json!({})),
    )
    .unwrap_err();
    assert!(cross_revision.contains("不属于本 root"), "{cross_revision}");
    assert_eq!(count(&connection), before, "被拒绝的边不得落库");

    let ancestor_edge = insert_evidence_edge(
        &connection,
        &evidence_edge("root-a", 2, "n-a3", "n-a1", json!({"why": "ancestor"})),
    )
    .unwrap();
    assert!(matches!(ancestor_edge, EvidenceInsert::Inserted(_)));
    // The database trigger must enforce the same rule even for non-repository writers.
    connection.execute(
        "INSERT INTO agent_evidence_edges(root_run_id,revision,from_node_id,to_node_id,kind,created_by_run_id) \
         VALUES('root-a',2,'n-a3','n-a2','derived_from','root-a')", [],
    ).unwrap();
    assert_eq!(count(&connection), before + 2);

    let conflict = insert_evidence_edge(
        &connection,
        &evidence_edge("root-a", 1, "n-a1", "n-a2", json!({"why": "other"})),
    )
    .unwrap_err();
    assert!(conflict.contains("evidence_edge_conflict"), "{conflict}");
    assert_eq!(count(&connection), before + 2);
    assert_eq!(
        list_evidence_edges_at_revision(&connection, "root-a", 1)
            .unwrap()
            .len(),
        1
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn evidence_snapshot_retains_ancestors_until_a_later_version_supersedes_them() {
    use super::evidence_graph::contract::{EvidenceNodeKind, EvidenceProvenance};
    use super::evidence_graph::store::{
        insert_evidence_node, list_effective_evidence_nodes_at_revision,
    };
    let (root, connection) = stage1a_connection("stage1a-snapshot");
    create_runs(&connection, &["snapshot-root", "other-root"]);
    let original = evidence_node(
        "old", "snapshot-root", 1, EvidenceNodeKind::Endpoint,
        EvidenceProvenance::Observed, json!({"value": 1}),
    );
    insert_evidence_node(&connection, &original).unwrap();
    let peer = evidence_node(
        "peer", "snapshot-root", 1, EvidenceNodeKind::Endpoint,
        EvidenceProvenance::Observed, json!({"value": "peer"}),
    );
    insert_evidence_node(&connection, &peer).unwrap();
    let other = evidence_node(
        "foreign", "other-root", 2, EvidenceNodeKind::Endpoint,
        EvidenceProvenance::Observed, json!({"value": "foreign"}),
    );
    insert_evidence_node(&connection, &other).unwrap();
    let successor = super::evidence_graph::contract::EvidenceNode {
        supersedes_id: original.id.clone(),
        ..evidence_node(
            "new", "snapshot-root", 2, EvidenceNodeKind::Endpoint,
            EvidenceProvenance::Observed, json!({"value": 2}),
        )
    };
    insert_evidence_node(&connection, &successor).unwrap();
    let ids = |revision| -> Vec<String> {
        list_effective_evidence_nodes_at_revision(&connection, "snapshot-root", revision)
            .unwrap().into_iter().map(|node| node.id).collect()
    };
    assert_eq!(ids(1), vec!["old", "peer"]);
    assert_eq!(ids(2), vec!["peer", "new"]);
    assert!(ids(3).is_empty(), "不存在的 revision 不能冒充证据快照");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn evidence_revision_cannot_retire_a_stronger_fact_with_a_weaker_claim() {
    use super::evidence_graph::contract::{EvidenceNodeKind, EvidenceProvenance};
    use super::evidence_graph::store::{insert_evidence_node, list_effective_evidence_nodes_at_revision};
    let (root, connection) = stage1a_connection("stage1a-provenance-supersession");
    create_runs(&connection, &["provenance-root"]);
    let observed = evidence_node(
        "observed", "provenance-root", 1, EvidenceNodeKind::Endpoint,
        EvidenceProvenance::Observed, json!({"url": "/known"}),
    );
    insert_evidence_node(&connection, &observed).unwrap();
    let inferred = super::evidence_graph::contract::EvidenceNode {
        supersedes_id: observed.id.clone(),
        ..evidence_node(
            "inferred", "provenance-root", 2, EvidenceNodeKind::Endpoint,
            EvidenceProvenance::Inferred, json!({"url": "/guessed"}),
        )
    };
    assert!(insert_evidence_node(&connection, &inferred).unwrap_err()
        .contains("provenance_downgrade"));
    // A pre-existing/externally imported SQLite row must not become a valid
    // snapshot just because it bypassed the repository's insert method.
    connection.execute(
        "INSERT INTO agent_evidence_nodes(id,root_run_id,revision,kind,provenance,natural_key_hash,\
         payload_json,created_by_run_id,supersedes_id) VALUES(?1,?2,2,'endpoint','inferred',?3,'{}',?2,?4)",
        rusqlite::params![inferred.id, inferred.root_run_id, inferred.natural_key_hash, observed.id],
    ).unwrap();
    assert!(list_effective_evidence_nodes_at_revision(&connection, "provenance-root", 2)
        .unwrap_err().contains("evidence_snapshot_invalid_supersession"));
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_evidence_edge_trigger_is_upgraded_without_losing_rows() {
    use super::evidence_graph::contract::{EvidenceNodeKind, EvidenceProvenance};
    use super::evidence_graph::store::insert_evidence_node;
    let (root, connection) = stage1a_connection("stage1a-legacy-edge-trigger");
    create_runs(&connection, &["legacy-root"]);
    for (id, revision) in [("legacy-old", 1), ("legacy-new", 2)] {
        insert_evidence_node(&connection, &evidence_node(
            id, "legacy-root", revision, EvidenceNodeKind::Endpoint,
            EvidenceProvenance::Observed, json!({"id": id}),
        )).unwrap();
    }
    connection.execute(
        "INSERT INTO agent_evidence_edges(root_run_id,revision,from_node_id,to_node_id,kind,created_by_run_id) \
         VALUES('legacy-root',1,'legacy-old','legacy-old','supports','legacy-root')", [],
    ).unwrap();
    connection.execute_batch(
        "DROP TRIGGER agent_evidence_edges_endpoints_insert;
         DROP TRIGGER agent_evidence_edges_endpoints_update;
         CREATE TRIGGER agent_evidence_edges_endpoints_insert
         BEFORE INSERT ON agent_evidence_edges BEGIN
             SELECT CASE WHEN NOT EXISTS (
                 SELECT 1 FROM agent_evidence_nodes n WHERE n.id=NEW.to_node_id
                   AND n.root_run_id=NEW.root_run_id AND n.revision=NEW.revision
             ) THEN RAISE(ABORT,'legacy same-revision guard') END;
         END;",
    ).unwrap();
    assert!(connection.execute(
        "INSERT INTO agent_evidence_edges(root_run_id,revision,from_node_id,to_node_id,kind,created_by_run_id) \
         VALUES('legacy-root',2,'legacy-new','legacy-old','supports','legacy-root')", [],
    ).is_err());
    drop(connection);

    let db_path = crate::db::initialize(&root).unwrap();
    crate::db::initialize(&root).unwrap(); // migration is repeatable
    let connection = crate::db::open(&db_path).unwrap();
    connection.execute(
        "INSERT INTO agent_evidence_edges(root_run_id,revision,from_node_id,to_node_id,kind,created_by_run_id) \
         VALUES('legacy-root',2,'legacy-new','legacy-old','supports','legacy-root')", [],
    ).unwrap();
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_evidence_edges WHERE root_run_id='legacy-root'", [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 2);
    let parent: i64 = connection.query_row(
        "SELECT parent_revision FROM agent_evidence_revisions \
         WHERE root_run_id='legacy-root' AND revision=2", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(parent, 1);
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn evidence_version_ancestry_rejects_a_late_inserted_non_ancestor() {
    use super::evidence_graph::contract::{EvidenceNodeKind, EvidenceProvenance};
    use super::evidence_graph::store::{
        evidence_revision_is_ancestor, insert_evidence_edge, insert_evidence_node,
        list_effective_evidence_nodes_at_revision,
    };
    let (root, connection) = stage1a_connection("stage1a-evidence-branch");
    create_runs(&connection, &["branch-root"]);
    let make = |id, revision| evidence_node(
        id, "branch-root", revision, EvidenceNodeKind::Endpoint,
        EvidenceProvenance::Observed, json!({"id": id}),
    );
    insert_evidence_node(&connection, &make("first-at-two", 2)).unwrap();
    insert_evidence_node(&connection, &make("late-one", 1)).unwrap();
    insert_evidence_node(&connection, &make("third", 3)).unwrap();
    let snapshot = list_effective_evidence_nodes_at_revision(&connection, "branch-root", 3)
        .unwrap().into_iter().map(|node| node.id).collect::<Vec<_>>();
    assert_eq!(snapshot, vec!["first-at-two", "third"]);
    assert!(!evidence_revision_is_ancestor(&connection, "branch-root", 3, 1).unwrap());
    assert!(evidence_revision_is_ancestor(&connection, "branch-root", 3, 2).unwrap());
    let non_ancestor_edge = evidence_edge(
        "branch-root", 3, "third", "late-one", json!({}),
    );
    assert!(insert_evidence_edge(&connection, &non_ancestor_edge).unwrap_err()
        .contains("不属于本 root/祖先"));
    assert!(connection.execute(
        "INSERT INTO agent_evidence_edges(root_run_id,revision,from_node_id,to_node_id,kind,created_by_run_id) \
         VALUES('branch-root',3,'third','late-one','supports','branch-root')", [],
    ).is_err(), "直接 SQL 同样不能引用非祖先");
    let successor = super::evidence_graph::contract::EvidenceNode {
        supersedes_id: "late-one".into(),
        ..make("invalid-successor", 4)
    };
    assert!(insert_evidence_node(&connection, &successor).unwrap_err()
        .contains("非祖先 revision"));
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// §4.9–§4.10 — a stored word the runtime does not know is an integrity error. It
/// may never be read back as the strongest provenance or as a fresh `prepared`.
#[test]
fn unknown_vocabulary_rows_fail_to_read() {
    use super::evidence_graph::store::list_evidence_nodes_at_revision;
    use super::multi_agent::assignment::load_assignment;
    let (root, connection) = stage1a_connection("stage1a-vocabulary");
    create_runs(&connection, &["root-a"]);
    insert_assignment_rows(&connection);
    connection
        .execute(
            "UPDATE agent_evidence_nodes SET provenance='telepathic' WHERE id='n-1'",
            [],
        )
        .unwrap();
    let error = list_evidence_nodes_at_revision(&connection, "root-a", 1).unwrap_err();
    assert!(error.contains("provenance"), "{error}");
    connection
        .execute("UPDATE agent_evidence_nodes SET provenance='observed' WHERE id='n-1'", [])
        .unwrap();
    connection
        .execute("UPDATE agent_evidence_nodes SET kind='what_is_this' WHERE id='n-1'", [])
        .unwrap();
    assert!(list_evidence_nodes_at_revision(&connection, "root-a", 1)
        .unwrap_err()
        .contains("kind"));
    connection
        .execute("UPDATE agent_evidence_nodes SET kind='endpoint' WHERE id='n-1'", [])
        .unwrap();

    connection
        .execute("UPDATE agent_assignments SET state='recycling' WHERE id='a-1'", [])
        .unwrap();
    let error = load_assignment(&connection, "a-1").unwrap_err();
    assert!(error.contains("state"), "{error}");
    connection
        .execute("UPDATE agent_assignments SET lane='wild' WHERE id='a-1'", [])
        .unwrap();
    assert!(load_assignment(&connection, "a-1")
        .unwrap_err()
        .contains("lane"));
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

