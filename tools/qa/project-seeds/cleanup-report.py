#!/usr/bin/env python3
"""Read-only cleanup receipt report. No network, model, or database mutation.

Example: cleanup-report.py --database /path/oviraptor.sqlite3 --project-id 3 \
    --run-id RUN --output-prefix /private/report/cleanup
Without --output-prefix, JSON and Markdown are written to stdout.
Counts keep domain groups, original index rows, ledger rows, and research work
separate. Deferred/unknown evidence is never presented as verified completion.
"""
from __future__ import annotations

import argparse
import collections
import datetime as dt
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import sqlite3
import sys
import tempfile
from urllib.parse import urlsplit, urlunsplit

REPORT_VERSION = 1
SHARED_ROOTS = frozenset({"cmecloud.cn", "myqcloud.com", "aliyuncs.com", "cloudfront.net", "akamaiedge.net", "akadns.net", "azureedge.net", "chinacloudsites.cn", "51job.com", "csdn.net", "163.com", "139site.com"})


def parsed(raw, fallback):
    try:
        return json.loads(raw) if isinstance(raw, str) else fallback
    except (ValueError, TypeError):
        return fallback


def safe_url(value):
    try:
        u = urlsplit(str(value))
        if u.scheme not in ("https", "http") or not u.hostname:
            return ""
        host = u.hostname
        if u.port:
            host += ":" + str(u.port)
        return urlunsplit((u.scheme, host, u.path, "", ""))
    except ValueError:
        return ""


def safe_text(value, limit=500):
    value = str(value or "")
    value = re.sub(r"https?://[^\s；，<>\"']+", lambda m: safe_url(m[0]) or "[URL removed]", value)
    value = re.sub(r"(?i)\b(api[-_]?key|authorization|cookie|password|secret|access[-_]?token)\s*[:=]\s*[^\s;,，；]+", r"\1=[REDACTED]", value)
    value = re.sub(r"\b[A-Za-z0-9_+/=-]{48,}\b", "[long value redacted]", value)
    return value[:limit]


def domain(value):
    v = str(value or "").strip().lower().rstrip(".")
    if v.startswith("*."):
        v = v[2:]
    if not v or len(v) > 253 or not re.fullmatch(r"[a-z0-9.-]+", v):
        return ""
    try:
        ipaddress.ip_address(v)
        return ""
    except ValueError:
        labels = v.split(".")
        return v if len(labels) > 1 and all(1 <= len(label) <= 63 and not label.startswith("-") and not label.endswith("-") for label in labels) else ""


def under(host, root):
    # An exact dotted suffix, never substring matching (notexample.cn != example.cn).
    return bool(host and root and (host == root or host.endswith("." + root)))


def provider(kind):
    if kind.startswith("ct_"):
        return "ct"
    return {"dns_cname": "dns", "public_ownership": "website", "organization": "organization"}.get(kind, kind or "unknown")


def reason_code(raw):
    s = str(raw or "").lower()
    if not s:
        return "reason_not_recorded"
    if "缺少主体" in s or "未找到完整法律主体" in s or "identity" in s:
        return "identity_not_found"
    if "dns status=3" in s or "nxdomain" in s:
        return "dns_nxdomain"
    if "servfail" in s or "dns status=2" in s:
        return "dns_servfail"
    for code in (403, 429, 412, 502, 503, 504, 404):
        if re.search(r"http(?:\s+|[^\w]+)" + str(code) + r"\b", s):
            return "http_" + str(code)
    if "timeout" in s or "timed out" in s or "时限" in s:
        return "timeout"
    if "请求失败" in s or "error sending request" in s:
        return "transport_unavailable"
    if "共享" in s or "shared" in s:
        return "shared_tenant"
    if "截断" in s or "超出" in s:
        return "response_incomplete"
    if "暂停" in s or "中断" in s:
        return "interrupted"
    return "other_recorded_error"


class Reader:
    def __init__(self, connection):
        self.c = connection
        self.tables = {r[0] for r in self.c.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        self.missing = set()

    def unavailable(self, table):
        return any(m == table or m.startswith(table + ":") for m in self.missing)

    def rows(self, table, required, where="", args=(), optional=()):
        if table not in self.tables:
            self.missing.add(table)
            return []
        columns = {r[1] for r in self.c.execute('PRAGMA table_info("' + table + '")')}
        absent = set(required) - columns
        absent.update(k for k in ("project_id", "run_id") if re.search(r"\b" + k + r"\b", where) and k not in columns)
        if absent:
            self.missing.add(table + ":" + ",".join(sorted(absent)))
            return []
        chosen = list(required) + [k for k in optional if k in columns]
        sql = "SELECT " + ",".join('"' + k + '"' for k in chosen) + ' FROM "' + table + '"'
        if where:
            sql += " WHERE " + where
        return [dict(zip(chosen, r)) for r in self.c.execute(sql, args)]


def trace_case(group, research, maximum=5):
    e = parsed(group.get("evidence_json"), {})
    facts = e.get("domainEvidence", []) if isinstance(e, dict) else []
    witnesses = []
    priority = {"official_site_operator": 0, "website_icp_declared": 1, "public_site_certificate": 2, "endpoint_icp": 3, "public_ct_subject": 4, "official_unit_relation": 5}
    ordered = sorted((f for f in facts if isinstance(f, dict)), key=lambda f: priority.get(f.get("source"), 6)) if isinstance(facts, list) else []
    identities = set()
    for f in ordered:
        if not isinstance(f, dict):
            continue
        identity = (f.get("host"), f.get("source"), f.get("organization"), f.get("sourceUrl"))
        if identity in identities:
            continue
        identities.add(identity)
        witnesses.append({"checkedAt": f.get("checkedAt"), "notAfter": f.get("notAfter"), "host": domain(f.get("host")), "organization": safe_text(f.get("organization"), 140), "source": safe_text(f.get("source"), 80), "sourceUrl": safe_url(f.get("sourceUrl", "")), "bodySha256": f.get("bodySha256") if re.fullmatch(r"[a-fA-F0-9]{64}", str(f.get("bodySha256", ""))) else None, "registryVerified": f.get("registryVerified") is True})
        if len(witnesses) >= maximum:
            break
    work = research.get(group["group_key"], [])
    return {"groupKey": safe_text(group["group_key"], 280), "domain": domain(group.get("domain")), "shared": bool(group.get("shared", False)), "state": group.get("state", "unknown"), "exclusionOrigin": group.get("_exclusion_origin") if group.get("state") == "excluded" else None, "advice": safe_text(group.get("advice"), 40), "reason": safe_text(group.get("reason")), "completionReason": safe_text(e.get("completionReason", "")) if isinstance(e, dict) else "", "evidenceWitnesses": witnesses, "research": [{"id": x["id"], "kind": x["kind"], "state": x["state"], "attempts": x.get("attempts", 0), "nextAt": x.get("next_at"), "reasonCode": reason_code(x.get("error")), "error": safe_text(x.get("error")), "sourceUrl": safe_url(parsed(x.get("result_json"), {}).get("sourceUrl", ""))} for x in sorted(work, key=lambda x: x["id"], reverse=True)[:maximum]]}


def build_report(connection, project_id, run_id=None, max_cases=5):
    """Read an already opened connection; caller must set query_only=ON."""
    if connection.execute("PRAGMA query_only").fetchone()[0] != 1:
        raise ValueError("report_requires_query_only_connection")
    rd = Reader(connection)
    runs = rd.rows("asset_cleanup_runs", ("id", "project_id"), "project_id=?", (project_id,), ("organization", "phase", "enabled", "raw_count", "ledger_count", "cursor", "reason", "created_at", "updated_at", "roots_json"))
    if run_id is not None:
        chosen = next((r for r in runs if r["id"] == run_id), None)
        if chosen is None:
            raise ValueError("cleanup_run_not_found_for_project")
    else:
        chosen = max(runs, key=lambda r: (r.get("created_at", 0), r["id"]), default=None)
    raw = rd.rows("asset_discovery_seeds", ("id", "host", "source", "source_ref"), "project_id=?", (project_id,))
    ledger = rd.rows("asset_project_seeds", ("id",), "project_id=?", (project_id,))
    report = {"reportVersion": REPORT_VERSION, "generatedAt": dt.datetime.now(dt.timezone.utc).isoformat(), "projectId": project_id, "run": chosen, "readOnly": True, "networkRequested": False, "modelRequested": False, "countUnits": {"groups": "domain or isolated fingerprint groups", "rawIndexRows": "original seed index rows, not asset count", "projectLedgerRows": "project seed ledger rows", "snapshotRows": "raw + ledger + provenance snapshot rows", "researchRows": "durable evidence work items"}, "notes": ["No key, cookie, raw certificate, full model request or full HTTP request is exported.", "Website ICP declarations are not official registry verification.", "An unresolved or deferred item is not verified ownership and not a conflict.", "Counts describe one consistent read snapshot; the running worker may progress later."]}
    report["inventory"] = {"currentRawIndexRows": len(raw) if not rd.unavailable("asset_discovery_seeds") else None, "currentProjectLedgerRows": len(ledger) if not rd.unavailable("asset_project_seeds") else None}
    if chosen is None:
        report.update({"availability": "no_cleanup_run", "completed": False, "groupStates": {}, "missingSchema": sorted(rd.missing)})
        return report
    run_id = chosen["id"]
    for key in ("organization", "reason"):
        if key in chosen:
            chosen[key] = safe_text(chosen[key])
    if "roots_json" in chosen:
        chosen["confirmedRoots"] = [domain(v) for v in parsed(chosen.pop("roots_json"), []) if domain(v)]
    groups = rd.rows("asset_cleanup_groups", ("group_key", "domain", "state"), "run_id=?", (run_id,), ("shared", "seed_count", "advice", "reason", "evidence_json"))
    snapshots = rd.rows("asset_cleanup_seeds", ("source_kind", "source_id", "host", "group_key"), "run_id=?", (run_id,), ("source",))
    work = rd.rows("asset_cleanup_research", ("id", "group_key", "kind", "state"), "run_id=?", (run_id,), ("attempts", "error", "next_at", "result_json"))
    batches = rd.rows("asset_cleanup_batches", ("id", "project_id", "domains_json", "restored"), "run_id=? AND project_id=?", (run_id, project_id), ("created_at",))
    rules = rd.rows("asset_discovery_domain_exclusions", ("domain", "enabled"), "project_id=?", (project_id,))
    live_domains = {domain(r["domain"]) for r in rules if r["enabled"] and domain(r["domain"])}
    batch_domains = set()
    batch_summaries = []
    for b in batches:
        domains = parsed(b["domains_json"], [])
        domains = [domain(x) for x in domains if domain(x)] if isinstance(domains, list) else []
        active = [d for d in domains if d in live_domains and not b["restored"]]
        batch_domains.update(active)
        batch_summaries.append({"batchId": b["id"], "domains": domains, "activeDomains": active, "restored": bool(b["restored"]), "createdAt": b.get("created_at")})
    raw_by_id = {r["id"]: r for r in raw}
    direct_ids = {i for i, r in raw_by_id.items() if any(under(domain(r["host"]), d) for d in batch_domains)}
    # Mirror the production provenance guard. A shared fingerprint is hidden only
    # when all its independent provenance rows are blocked, never from one edge.
    edges = rd.rows("asset_discovery_excluded_evidence", ("host", "source", "source_ref", "domain"), "project_id=?", (project_id,))
    provenance = rd.rows("asset_discovery_seed_evidence", ("host", "source", "source_ref"), "project_id=?", (project_id,))
    by_seed = collections.defaultdict(set)
    for e in provenance:
        by_seed[(e["host"], e["source"])].add(e["source_ref"])
    blocked = {(e["host"], e["source"], e["source_ref"]) for e in edges if domain(e["domain"]) in live_domains}
    new_edges = {(e["host"], e["source"], e["source_ref"]) for e in edges if domain(e["domain"]) in batch_domains}
    provenance_ids = set()
    provenance_available = not any(rd.unavailable(k) for k in ("asset_discovery_seed_evidence", "asset_discovery_excluded_evidence"))
    if provenance_available:
        for i, r in raw_by_id.items():
            edge = (r["host"], r["source"], r["source_ref"])
            if edge in new_edges and all((r["host"], r["source"], ref) in blocked for ref in by_seed[(r["host"], r["source"])]):
                provenance_ids.add(i)
    raw_available = not rd.unavailable("asset_discovery_seeds")
    unsafe_roots = sorted(d for d in batch_domains if d in SHARED_ROOTS)
    snapshot_raw_ids = {s["source_id"] for s in snapshots if s["source_kind"] == "raw"}
    covered_snapshot_ids = (direct_ids | provenance_ids) & snapshot_raw_ids
    report["exclusions"] = {"newRunBatchCount": len(batches), "activeBatchDomainCount": len(batch_domains), "restoredBatchCount": sum(bool(b["restored"]) for b in batches), "historicalEnabledDomainRulesOutsideRun": len(live_domains - batch_domains), "coveredOriginalRawIndexRowsDirect": len(direct_ids) if raw_available else None, "coveredOriginalRawIndexRowsViaProvenance": len(provenance_ids - direct_ids) if provenance_available and raw_available else None, "coveredOriginalRawIndexRowsUnion": len(direct_ids | provenance_ids) if provenance_available and raw_available else None, "coverageMethod": "current project raw rows, exact domain or dotted suffix plus production all-independent-provenance guard; no physical deletion", "provenanceCoverageAvailable": provenance_available, "coveredSnapshotOriginalRawIndexRows": len(covered_snapshot_ids) if provenance_available and raw_available and not rd.unavailable("asset_cleanup_seeds") else None, "sharedRootRuleWarnings": unsafe_roots, "batches": sorted(batch_summaries, key=lambda b: (b.get("createdAt") or 0, b["batchId"]), reverse=True)}
    states = collections.Counter(g["state"] for g in groups)
    report["availability"] = "available"
    report["groupStates"] = dict(sorted(states.items()))
    report["snapshot"] = {"declaredRawIndexRows": chosen.get("raw_count"), "declaredProjectLedgerRows": chosen.get("ledger_count"), "actualSnapshotRows": len(snapshots), "snapshotRowsByKind": dict(sorted(collections.Counter(s["source_kind"] for s in snapshots).items())), "groupCount": len(groups), "snapshotProcessedCursor": chosen.get("cursor")}
    lookup = {g["group_key"]: g for g in groups}
    work_by_group = collections.defaultdict(list)
    work_counts = collections.defaultdict(collections.Counter)
    err_counts = collections.Counter()
    err_samples = {}
    group_reason_counts = collections.Counter()
    for g in groups:
        ev = parsed(g.get("evidence_json"), {})
        comp = ev.get("completionReason") if isinstance(ev, dict) else None
        if g["state"] in ("deferred", "evidence", "held", "conflict"):
            group_reason_counts[(g["state"], safe_text(comp or g.get("reason") or "reason_not_recorded"))] += 1
    for w in work:
        key = w["group_key"]
        if key not in lookup and not (key == "__organization" and w["kind"] == "organization"):
            continue
        work_by_group[key].append(w)
        kind = w["kind"]
        work_counts[kind]["rows"] += 1
        work_counts[kind][w["state"]] += 1
        if w["state"] in ("pending", "retry", "running"):
            eligible = w["kind"] == "organization" or lookup.get(key, {}).get("state") in ("pending", "evidence")
            work_counts[kind]["activeUnfinished" if eligible else "parkedUnfinished"] += 1
        work_counts[kind]["attempted" if w.get("attempts", 0) > 0 else "neverAttempted"] += 1
        if w.get("error"):
            code = reason_code(w["error"])
            ek = (provider(kind), kind, code)
            err_counts[ek] += 1
            err_samples.setdefault(ek, safe_text(w["error"]))
    report["research"] = {"byKind": {k: dict(sorted(v.items())) for k, v in sorted(work_counts.items())}, "firstPassGroupsAttempted": sum(any(w.get("attempts", 0) > 0 for w in work_by_group[g["group_key"]]) for g in groups), "firstPassGroupsNeverAttempted": sum(not any(w.get("attempts", 0) > 0 for w in work_by_group[g["group_key"]]) for g in groups), "groupsWithExhaustedWork": sum(any(w["state"] == "exhausted" for w in work_by_group[g["group_key"]]) for g in groups), "errorsByProviderKindReason": [{"provider": p, "kind": k, "reason": r, "workItems": n, "exampleError": err_samples[(p, k, r)]} for (p, k, r), n in err_counts.most_common()], "remainingReasons": [{"state": s, "reason": r, "groups": n} for (s, r), n in group_reason_counts.most_common()]}
    # Case limits apply independently to each state; unknown data remains unknown.
    report["cases"] = {}
    for g in groups:
        if g["state"] == "excluded":
            g["_exclusion_origin"] = "active_this_run_batch" if domain(g.get("domain")) in batch_domains else "historical_or_other_rule"
    for name, included in (("excluded", {"excluded"}), ("retained", {"retained"}), ("unresolved", {"evidence", "deferred", "held"}), ("conflicts", {"conflict"})):
        candidates = sorted((g for g in groups if g["state"] in included), key=lambda g: (g.get("_exclusion_origin") != "active_this_run_batch" if g["state"] == "excluded" else False, g.get("domain", ""), g["group_key"]))
        limit = max_cases
        report["cases"][name] = {"totalGroups": len(candidates), "truncated": len(candidates) > limit, "items": [trace_case(g, work_by_group) for g in candidates[:limit]]}
    report["conflictGroups"] = [trace_case(g, work_by_group, maximum=3) for g in sorted(groups, key=lambda g: (g.get("domain", ""), g["group_key"])) if g["state"] == "conflict"]
    report["completed"] = chosen.get("phase") == "completed" and not any(states[s] for s in ("pending", "evidence", "deferred", "conflict"))
    report["passFinished"] = chosen.get("phase") in ("completed", "completed_with_gaps", "review_ready")
    report["verifiedResolvedGroups"] = sum(states[s] for s in ("excluded", "retained", "restored"))
    report["unresolvedGroups"] = sum(states[s] for s in ("pending", "evidence", "deferred", "held", "conflict"))
    report["missingSchema"] = sorted(rd.missing)
    if any(rd.unavailable(t) for t in ("asset_cleanup_groups", "asset_cleanup_research", "asset_cleanup_seeds", "asset_discovery_seeds", "asset_project_seeds", "asset_cleanup_batches", "asset_discovery_domain_exclusions")):
        report["completed"] = False
        report["passFinished"] = False
        report["availability"] = "partial_schema"
    return report


def database_stamp(path):
    """Identity/change detector for the no-WAL immutable reader boundary."""
    stat = path.stat()
    return {"device": stat.st_dev, "inode": stat.st_ino, "size": stat.st_size,
            "mtimeNs": stat.st_mtime_ns, "ctimeNs": stat.st_ctime_ns}


def wal_present(path):
    # A zero-byte WAL (or an unresolved WAL symlink) still forbids immutable.
    # Checking only a positive file size could hide an active writer's commit.
    return os.path.lexists(os.fspath(path) + "-wal")


def read_report(path, project_id, run_id=None, max_cases=5):
    path = Path(path).expanduser().resolve(strict=True)
    immutable = not wal_present(path)
    before = database_stamp(path) if immutable else None
    if immutable and wal_present(path):
        raise ValueError("cleanup_report_wal_appeared_before_immutable_read")
    # Live/present WAL always uses SQLite's normal read transaction. With no WAL,
    # immutable avoids trying to create SHM beside a checkpointed read-only DB;
    # accept its report only if both DB identity and no-WAL state stayed stable.
    uri = path.as_uri() + "?mode=ro" + ("&immutable=1" if immutable else "")
    c = sqlite3.connect(uri, uri=True, timeout=5)
    try:
        c.execute("PRAGMA query_only=ON")
        c.execute("BEGIN")
        report = build_report(c, project_id, run_id, max_cases)
        c.rollback()
        after = database_stamp(path) if immutable else None
        if immutable and (wal_present(path) or before != after):
            raise ValueError("cleanup_report_database_or_wal_changed_during_immutable_read")
        report["database"] = {"name": path.name, "pathHashSha256": hashlib.sha256(os.fsencode(path)).hexdigest(), "uriMode": "ro", "queryOnly": True,
                              "immutable": immutable, "snapshotPolicy": "stable_no_wal_immutable" if immutable else "live_wal_read_transaction",
                              "fileStampBefore": before, "fileStampAfter": after}
        return report
    finally:
        c.close()


def markdown(report):
    def cell(v):
        return str(v if v is not None else "未获取").replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("[", "&#91;").replace("]", "&#93;").replace("|", "\\|").replace("\n", " ")
    run = report.get("run") or {}
    lines = ["# 项目种子核验报告", "", f"项目 {report['projectId']} · Run {cell(run.get('id'))} · {report['generatedAt']}", "", f"阶段：{cell(run.get('phase'))}；全部核验完成：{'是' if report.get('completed') else '否'}。", "", "仅只读统计，不联网、不调用模型、不修改数据库。", "", "| 计数口径 | 数量 |", "| --- | ---: |"]
    for label, value in (("原始种子索引行（当前）", report.get("inventory", {}).get("currentRawIndexRows")), ("项目种子台账行（当前）", report.get("inventory", {}).get("currentProjectLedgerRows")), ("原始索引行（任务快照声明）", report.get("snapshot", {}).get("declaredRawIndexRows")), ("台账行（任务快照声明）", report.get("snapshot", {}).get("declaredProjectLedgerRows")), ("快照总行（含来源）", report.get("snapshot", {}).get("actualSnapshotRows")), ("域名/独立指纹组", report.get("snapshot", {}).get("groupCount"))):
        lines.append(f"| {label} | {cell(value)} |")
    lines += ["", "## 组状态", "", "| 状态 | 组数 |", "| --- | ---: |"]
    lines += [f"| {cell(k)} | {v} |" for k, v in report.get("groupStates", {}).items()]
    ex = report.get("exclusions", {})
    lines += ["", "## 可撤销排除", "", f"本轮批次 {cell(ex.get('newRunBatchCount'))}；当前生效域名规则 {cell(ex.get('activeBatchDomainCount'))}；覆盖原始索引行 {cell(ex.get('coveredOriginalRawIndexRowsUnion'))}（直接域名匹配 {cell(ex.get('coveredOriginalRawIndexRowsDirect'))}，独立来源核验后额外覆盖 {cell(ex.get('coveredOriginalRawIndexRowsViaProvenance'))}）。以上不是资产删除数。", "", "## 补证进度", "", "| 来源任务 | 已尝试条目 | 尚未尝试 | 成功 | 耗尽 | 跳过 |", "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for kind, counts in report.get("research", {}).get("byKind", {}).items():
        lines.append(f"| {cell(kind)} | {counts.get('attempted', 0)} | {counts.get('neverAttempted', 0)} | {counts.get('done', 0)} | {counts.get('exhausted', 0)} | {counts.get('skipped', 0)} |")
    lines += ["", "## 剩余原因", ""]
    for item in report.get("research", {}).get("remainingReasons", []):
        lines.append(f"- {item['groups']} 组 · {cell(item['state'])} · {cell(item['reason'])}")
    lines += ["", "## 去重错误", "", "| Provider / Kind | 原因 | 工作条目数 | 示例 |", "| --- | --- | ---: | --- |"]
    for item in report.get("research", {}).get("errorsByProviderKindReason", []):
        lines.append(f"| {cell(item['provider'])} / {cell(item['kind'])} | {cell(item['reason'])} | {item['workItems']} | {cell(item['exampleError'])} |")
    for name, bundle in report.get("cases", {}).items():
        lines += ["", f"## {name} 案例（{len(bundle['items'])} / {bundle['totalGroups']} 组）", ""]
        for item in bundle["items"]:
            lines.append(f"- **{cell(item['domain'] or item['groupKey'])}** · {cell(item['state'])} · {cell(item['completionReason'] or item['reason'])}")
            for fact in item["evidenceWitnesses"]:
                lines.append(f"  - {cell(fact['source'])} · {cell(fact['organization'])} · {cell(fact['sourceUrl'])} · 正文 SHA-256 {cell(fact['bodySha256'])}")
    lines += ["", "## 限制", "", "- 网站备案声明不等于官方备案主体查询结果。缺证、耗尽和来源不可用不等于无关，亦不等于冲突。", "- 同域规则按完整域名或点分隔后缀匹配。共享证书保留独立来源，不因单条来源被排除就整体隐藏。", "- deferred 表示本轮有限查证仍有缺口，不会记为已经核验完成。"]
    if report.get("missingSchema"):
        lines.append("- 未获取的表/字段：" + ", ".join(report["missingSchema"]) + "。相关计数不能作为完整性证明。")
    return "\n".join(lines) + "\n"


def private_write(path, content):
    path = Path(path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, temp = tempfile.mkstemp(prefix=".cleanup-report-", dir=path.parent)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as out:
            out.write(content)
        os.replace(temp, path)
    finally:
        if os.path.exists(temp):
            os.unlink(temp)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--database", required=True, type=Path)
    ap.add_argument("--project-id", required=True, type=int)
    ap.add_argument("--run-id")
    ap.add_argument("--max-cases", type=int, default=5)
    ap.add_argument("--output-prefix", type=Path, help="write PREFIX.json and PREFIX.md, each mode 0600")
    ap.add_argument("--format", choices=("json", "markdown", "both"), default="both")
    args = ap.parse_args()
    if not 0 <= args.max_cases <= 100:
        ap.error("--max-cases must be between 0 and 100")
    try:
        report = read_report(args.database, args.project_id, args.run_id, args.max_cases)
    except (ValueError, OSError, sqlite3.Error) as exc:
        print("cleanup report failed: " + safe_text(exc), file=sys.stderr)
        return 1
    j = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    m = markdown(report)
    if args.output_prefix:
        if args.format in ("json", "both"):
            private_write(str(args.output_prefix) + ".json", j)
        if args.format in ("markdown", "both"):
            private_write(str(args.output_prefix) + ".md", m)
    else:
        if args.format in ("json", "both"):
            sys.stdout.write(j)
        if args.format in ("markdown", "both"):
            if args.format == "both":
                sys.stdout.write("\n--- MARKDOWN REPORT ---\n\n")
            sys.stdout.write(m)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
