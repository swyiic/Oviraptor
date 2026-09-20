//! Persistence for the shared evidence graph (Stage 1A §5.3–§5.4, §6).
//!
//! Node identity is `(root_run_id, revision, natural_key_hash)`, so a replay of the
//! same observation is a no-op rather than a duplicate fact. Edges are refused when
//! either endpoint is missing: a dangling relation would look like evidence.

// Stage 1A declares the contract and its storage only; the scheduler, the child
// runs and the review gate that consume them land in the next stages. Every item
// here is exercised by the Stage 1A tests, so the reachability warning is expected.
#![allow(dead_code)]
use super::contract::{
    EvidenceEdge, EvidenceEdgeKind, EvidenceNode, EvidenceNodeKind, EvidenceProvenance,
};
use crate::agent_runtime::secrets::redact_json;
use crate::agent_runtime::store::stable_hash;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;

const NODE_COLUMNS: &str = "id,root_run_id,revision,kind,provenance,natural_key_hash,payload_json,\
     artifact_refs_json,created_by_run_id,supersedes_id,created_at";
const EDGE_COLUMNS: &str = "id,root_run_id,revision,from_node_id,to_node_id,kind,payload_json,\
     created_by_run_id,created_at";

/// Content-addressed natural key: same root, kind and identifying text always
/// produce the same hash, so a replay deduplicates instead of adding a second fact.
pub fn evidence_natural_key(root_run_id: &str, kind: EvidenceNodeKind, identity: &str) -> String {
    stable_hash(&format!("{root_run_id}|{}|{identity}", kind.as_str()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvidenceInsert {
    Inserted(String),
    /// The same node already exists at this revision.
    Existing(String),
}

fn node_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EvidenceNode> {
    Ok(EvidenceNode {
        id: row.get(0)?,
        root_run_id: row.get(1)?,
        revision: row.get(2)?,
        kind: EvidenceNodeKind::parse(&row.get::<_, String>(3)?),
        provenance: EvidenceProvenance::parse(&row.get::<_, String>(4)?),
        natural_key_hash: row.get(5)?,
        payload: serde_json::from_str(&row.get::<_, String>(6).unwrap_or_default())
            .unwrap_or_else(|_| JsonValue::Object(serde_json::Map::new())),
        artifact_refs: serde_json::from_str(&row.get::<_, String>(7).unwrap_or_default())
            .unwrap_or_default(),
        created_by_run_id: row.get(8)?,
        supersedes_id: row.get(9)?,
        created_at: row.get(10)?,
    })
}

/// Store one node. The payload and the artifact references go through the runtime's
/// existing redaction before they touch SQL — there is no second scrubber here.
pub fn insert_evidence_node(
    connection: &Connection,
    node: &EvidenceNode,
) -> Result<EvidenceInsert, String> {
    if node.id.trim().is_empty() {
        return Err("evidence node 缺少 id".to_string());
    }
    if node.natural_key_hash.trim().is_empty() {
        return Err("evidence node 缺少 natural_key_hash".to_string());
    }
    let payload = redact_json(&node.payload).to_string();
    let refs = redact_json(&serde_json::json!(node.artifact_refs)).to_string();
    let existing: Option<String> = connection
        .query_row(
            "SELECT id FROM agent_evidence_nodes WHERE root_run_id=?1 AND revision=?2 AND natural_key_hash=?3",
            params![node.root_run_id, node.revision, node.natural_key_hash],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("无法读取已有 evidence node：{error}"))?;
    if let Some(found) = existing {
        return Ok(EvidenceInsert::Existing(found));
    }
    match connection.execute(
        "INSERT INTO agent_evidence_nodes(id,root_run_id,revision,kind,provenance,natural_key_hash,\
         payload_json,artifact_refs_json,created_by_run_id,supersedes_id,created_at) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,datetime('now','localtime'))",
        params![
            node.id,
            node.root_run_id,
            node.revision,
            node.kind.as_str(),
            node.provenance.as_str(),
            node.natural_key_hash,
            payload,
            refs,
            node.created_by_run_id,
            node.supersedes_id
        ],
    ) {
        Ok(_) => Ok(EvidenceInsert::Inserted(node.id.clone())),
        Err(error) if error.to_string().contains("UNIQUE") => Ok(EvidenceInsert::Existing(
            connection
                .query_row(
                    "SELECT id FROM agent_evidence_nodes WHERE root_run_id=?1 AND revision=?2 AND natural_key_hash=?3",
                    params![node.root_run_id, node.revision, node.natural_key_hash],
                    |row| row.get(0),
                )
                .map_err(|error| format!("无法确认重放的 evidence node：{error}"))?,
        )),
        Err(error) => Err(format!("无法写入 evidence node：{error}")),
    }
}

pub fn load_evidence_node(
    connection: &Connection,
    id: &str,
) -> Result<Option<EvidenceNode>, String> {
    connection
        .query_row(
            &format!("SELECT {NODE_COLUMNS} FROM agent_evidence_nodes WHERE id=?1"),
            [id],
            node_from_row,
        )
        .optional()
        .map_err(|error| format!("无法读取 evidence node：{error}"))
}

pub fn list_evidence_nodes_at_revision(
    connection: &Connection,
    root_run_id: &str,
    revision: i64,
) -> Result<Vec<EvidenceNode>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {NODE_COLUMNS} FROM agent_evidence_nodes WHERE root_run_id=?1 AND revision=?2 ORDER BY created_at,id"
        ))
        .map_err(|error| format!("无法准备 evidence node 查询：{error}"))?;
    let rows = statement
        .query_map(params![root_run_id, revision], node_from_row)
        .map_err(|error| format!("无法读取 evidence node 列表：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析 evidence node 列表：{error}"))
}

fn edge_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EvidenceEdge> {
    Ok(EvidenceEdge {
        id: row.get(0)?,
        root_run_id: row.get(1)?,
        revision: row.get(2)?,
        from_node_id: row.get(3)?,
        to_node_id: row.get(4)?,
        kind: EvidenceEdgeKind::parse(&row.get::<_, String>(5)?),
        payload: serde_json::from_str(&row.get::<_, String>(6).unwrap_or_default())
            .unwrap_or_else(|_| JsonValue::Object(serde_json::Map::new())),
        created_by_run_id: row.get(7)?,
        created_at: row.get(8)?,
    })
}

/// Store one relation. Both endpoints must already exist: a hypothesis that points
/// at nothing would otherwise be quoted as support later (§9.1).
pub fn insert_evidence_edge(
    connection: &Connection,
    edge: &EvidenceEdge,
) -> Result<EvidenceInsert, String> {
    for (side, id) in [
        ("from_node_id", edge.from_node_id.as_str()),
        ("to_node_id", edge.to_node_id.as_str()),
    ] {
        let present: Option<String> = connection
            .query_row(
                "SELECT id FROM agent_evidence_nodes WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("无法检查 evidence 端点：{error}"))?;
        if present.is_none() {
            return Err(format!("evidence 边的{side}不存在，已拒绝写入：{id}"));
        }
    }
    let payload = redact_json(&edge.payload).to_string();
    let existing: Option<i64> = connection
        .query_row(
            "SELECT id FROM agent_evidence_edges WHERE root_run_id=?1 AND revision=?2 AND from_node_id=?3 AND to_node_id=?4 AND kind=?5",
            params![edge.root_run_id, edge.revision, edge.from_node_id, edge.to_node_id, edge.kind.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("无法读取已有 evidence 边：{error}"))?;
    if let Some(found) = existing {
        return Ok(EvidenceInsert::Existing(found.to_string()));
    }
    match connection.execute(
        "INSERT INTO agent_evidence_edges(root_run_id,revision,from_node_id,to_node_id,kind,payload_json,\
         created_by_run_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,datetime('now','localtime'))",
        params![
            edge.root_run_id,
            edge.revision,
            edge.from_node_id,
            edge.to_node_id,
            edge.kind.as_str(),
            payload,
            edge.created_by_run_id
        ],
    ) {
        Ok(_) => Ok(EvidenceInsert::Inserted(
            edge.id.to_string(),
        )),
        Err(error) if error.to_string().contains("UNIQUE") => Ok(EvidenceInsert::Existing(
            connection
                .query_row(
                    "SELECT id FROM agent_evidence_edges WHERE root_run_id=?1 AND revision=?2 AND from_node_id=?3 AND to_node_id=?4 AND kind=?5",
                    params![edge.root_run_id, edge.revision, edge.from_node_id, edge.to_node_id, edge.kind.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .map(|found| found.to_string())
                .map_err(|error| format!("无法确认重放的 evidence 边：{error}"))?,
        )),
        Err(error) => Err(format!("无法写入 evidence 边：{error}")),
    }
}

pub fn list_evidence_edges_at_revision(
    connection: &Connection,
    root_run_id: &str,
    revision: i64,
) -> Result<Vec<EvidenceEdge>, String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {EDGE_COLUMNS} FROM agent_evidence_edges WHERE root_run_id=?1 AND revision=?2 ORDER BY id"
        ))
        .map_err(|error| format!("无法准备 evidence 边查询：{error}"))?;
    let rows = statement
        .query_map(params![root_run_id, revision], edge_from_row)
        .map_err(|error| format!("无法读取 evidence 边列表：{error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法解析 evidence 边列表：{error}"))
}
