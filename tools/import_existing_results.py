#!/usr/bin/env python3
"""将已有 P1/P2/P3 probe CSV 增量导入 Oviraptor SQLite。

只新增/更新数据库，不删除或修改源 CSV。脚本可重复执行；资产、项目关联和
首次发现事件均按唯一键去重。
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import sqlite3
import sys
from pathlib import Path


def default_db() -> Path:
    if sys.platform == "darwin":
        return Path.home() / "oviraptor/oviraptor.sqlite3"
    if os.name == "nt":
        return Path.home() / "oviraptor/oviraptor.sqlite3"
    return Path.home() / "oviraptor/oviraptor.sqlite3"


def first(row: dict[str, str], *names: str) -> str:
    for name in names:
        value = (row.get(name) or "").strip()
        if value:
            return value
    return ""


def digest(values: list[str]) -> str:
    return hashlib.sha256("\x1f".join(values).encode("utf-8", "replace")).hexdigest()[:16]


def canonical_identity(link: str, host: str, protocol: str, ip: str, port: str, fallback: str) -> str:
    if link.strip():
        return link.strip().rstrip("/").lower()
    if host.strip():
        return host.strip().rstrip("/").lower()
    if ip.strip() or port.strip():
        return f"{protocol.strip()}|{ip.strip()}|{port.strip()}".lower()
    return fallback.strip().rstrip("/").lower()


def import_file(connection: sqlite3.Connection, project_id: int, run_id: int, path: Path) -> tuple[int, int]:
    imported = invalid = 0
    with path.open(encoding="utf-8-sig", newline="") as source:
        reader = csv.DictReader(source)
        for row in reader:
            company = first(row, "company")
            host = first(row, "host")
            link = first(row, "link")
            ip = first(row, "ip")
            port = first(row, "port")
            protocol = first(row, "protocol")
            source_key = first(row, "asset_key")
            raw_key = source_key or f"{link}|{host}|{ip}|{port}"
            if not raw_key.strip("|"):
                invalid += 1
                continue
            canonical_key = canonical_identity(link, host, protocol, ip, port, raw_key)
            existing = connection.execute(
                """SELECT a.asset_key FROM project_assets pa JOIN assets a ON a.id=pa.asset_id
                   WHERE pa.project_id=? AND a.canonical_key=? AND pa.is_deleted=0
                   ORDER BY pa.asset_id LIMIT 1""",
                (project_id, canonical_key),
            ).fetchone()
            asset_key = existing[0] if existing else f"{company}\x1f{raw_key}"
            domain = first(row, "domain")
            title = first(row, "probe_title", "title")
            status_code = first(row, "probe_status_code", "status_code")
            probe_outcome = first(row, "probe_outcome")
            probe_entry_state = first(row, "probe_entry_state")
            review_tier = first(row, "review_tier")
            content_category = first(row, "content_category")
            score = first(row, "score")
            state_hash = digest([host, link, ip, port, protocol, domain, review_tier, score])
            probe_hash = digest([probe_outcome, probe_entry_state, status_code, title, content_category])
            extra_json = json.dumps({key: value for key, value in row.items() if value}, ensure_ascii=False, separators=(",", ":"))

            connection.execute(
                """
                INSERT INTO assets(asset_key,company,host,link,ip,port,protocol,domain,title,status_code,
                  probe_outcome,probe_entry_state,review_tier,content_category,score,state_hash,probe_hash,
                  canonical_key,extra_json,last_alive)
                VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,CASE WHEN ?='alive_clean' THEN datetime('now','localtime') ELSE NULL END)
                ON CONFLICT(asset_key) DO UPDATE SET
                  company=excluded.company,host=excluded.host,link=excluded.link,ip=excluded.ip,port=excluded.port,
                  protocol=excluded.protocol,domain=excluded.domain,title=excluded.title,status_code=excluded.status_code,
                  probe_outcome=excluded.probe_outcome,probe_entry_state=excluded.probe_entry_state,
                  review_tier=excluded.review_tier,content_category=excluded.content_category,score=excluded.score,
                  state_hash=excluded.state_hash,probe_hash=excluded.probe_hash,
                  canonical_key=excluded.canonical_key,
                  extra_json=json_patch(assets.extra_json,excluded.extra_json),last_seen=datetime('now','localtime'),
                  last_alive=CASE WHEN excluded.probe_outcome='alive_clean' THEN datetime('now','localtime') ELSE assets.last_alive END
                """,
                (asset_key, company, host, link, ip, port, protocol, domain, title, status_code,
                 probe_outcome, probe_entry_state, review_tier, content_category, score, state_hash,
                 probe_hash, canonical_key, extra_json, probe_outcome),
            )
            asset_id = connection.execute("SELECT id FROM assets WHERE asset_key=?", (asset_key,)).fetchone()[0]
            association = connection.execute(
                "INSERT OR IGNORE INTO project_assets(project_id,asset_id,last_run_id) VALUES(?,?,?)",
                (project_id, asset_id, run_id),
            )
            if association.rowcount:
                connection.execute(
                    "INSERT INTO asset_events(project_id,asset_id,run_id,event_type,summary) VALUES(?,?,?,'new','从历史 P1/P2/P3 探测结果导入')",
                    (project_id, asset_id, run_id),
                )
            else:
                connection.execute(
                    "UPDATE project_assets SET last_seen=datetime('now','localtime'),last_run_id=? WHERE project_id=? AND asset_id=?",
                    (run_id, project_id, asset_id),
                )
            imported += 1
    return imported, invalid


def main() -> int:
    parser = argparse.ArgumentParser(description="导入 Oviraptor 历史 P1/P2/P3 探测结果")
    parser.add_argument("--db", type=Path, default=default_db())
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--project-name", default="中国移动历史资产")
    args = parser.parse_args()

    files = sorted(
        path for path in args.input_dir.glob("P[123]_*.csv")
        if any(marker in path.name for marker in ("alive_clean", "blocked_content", "unreachable"))
    )
    if not files:
        parser.error(f"未在 {args.input_dir} 找到 P1/P2/P3 探测 CSV")
    if not args.db.exists():
        parser.error(f"数据库不存在：{args.db}")

    connection = sqlite3.connect(args.db, timeout=60)
    connection.execute("PRAGMA foreign_keys=ON")
    connection.execute("PRAGMA busy_timeout=60000")
    try:
        connection.execute(
            "INSERT OR IGNORE INTO projects(name,description) VALUES(?,?)",
            (args.project_name, "由既有 P1/P2/P3 存活探测结果增量导入；源 CSV 保持不变"),
        )
        project_id = connection.execute("SELECT id FROM projects WHERE name=?", (args.project_name,)).fetchone()[0]
        connection.execute(
            "INSERT INTO runs(project_id,name,pipeline,status,stage,progress,total,config_snapshot,output_dir,started_at) VALUES(?,?,'historical_import','running','import',0,?,'{}',?,datetime('now','localtime'))",
            (project_id, "导入既有 P1/P2/P3 探测结果", len(files), str(args.input_dir)),
        )
        run_id = connection.execute("SELECT last_insert_rowid()").fetchone()[0]
        connection.commit()

        total = invalid = 0
        for index, path in enumerate(files, 1):
            imported, bad = import_file(connection, project_id, run_id, path)
            total += imported
            invalid += bad
            connection.execute(
                "UPDATE runs SET progress=?,processed=? WHERE id=?",
                (index / len(files) * 100, total, run_id),
            )
            connection.execute(
                "INSERT INTO logs(run_id,level,stage,message) VALUES(?,'info','import',?)",
                (run_id, f"{path.name}：导入 {imported}，无效 {bad}"),
            )
            connection.commit()
            print(f"[{index}/{len(files)}] {path.name}: {imported}")

        connection.execute(
            "UPDATE runs SET status='completed',stage='completed',progress=100,processed=?,finished_at=datetime('now','localtime') WHERE id=?",
            (total, run_id),
        )
        connection.execute(
            "INSERT INTO logs(run_id,level,stage,message) VALUES(?,'info','completed',?)",
            (run_id, f"历史导入完成：读取 {total} 条，无效 {invalid} 条"),
        )
        connection.commit()
        unique_count = connection.execute(
            "SELECT COUNT(*) FROM project_assets WHERE project_id=? AND is_deleted=0", (project_id,)
        ).fetchone()[0]
        print(json.dumps({"project_id": project_id, "read": total, "invalid": invalid, "unique_assets": unique_count}, ensure_ascii=False))
        return 0
    except Exception as exc:
        connection.rollback()
        try:
            connection.execute(
                "UPDATE runs SET status='failed',stage='failed',error=?,finished_at=datetime('now','localtime') WHERE id=?",
                (str(exc), locals().get("run_id", 0)),
            )
            connection.commit()
        except Exception:
            pass
        raise
    finally:
        connection.close()


if __name__ == "__main__":
    raise SystemExit(main())
