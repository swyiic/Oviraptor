#!/usr/bin/env python3
"""离线重分层现有 FOFA 候选，不访问 FOFA、不修改原 output。

输出重点人工复核集、证书簇、名称域名簇和失败查询重试计划。
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


BASE_DIR = Path(__file__).resolve().parent
ILLEGAL_XML_RE = re.compile(r"[\x00-\x08\x0B\x0C\x0E-\x1F]")
ACTIVE_STATUSES = {"200", "201", "202", "204", "206", "301", "302", "303", "307", "308", "401", "403"}
NAME_MARKERS = ("标题全称:", "正文全称:", "独有标题:", "独有正文:")
DERIVED_FIELDS = [
    "review_tier", "attribution_reason", "activity_state", "evidence_types",
    "company_match_count",
]


def clean(value: Any) -> str:
    return ILLEGAL_XML_RE.sub("", "" if value is None else str(value)).strip()


def evidence_types(evidence: str) -> str:
    mapping = {
        "证书组织:": "cert_org", "证书序列号:": "cert_serial",
        "标题全称:": "name_title", "正文全称:": "name_body",
        "独有标题:": "unique_title", "独有正文:": "unique_body",
        "响应头哈希:": "header_hash", "弱JARM:": "jarm",
        "Banner结构指纹:": "banner_fid", "图标哈希:": "icon_hash",
        "FOFA站点特征:": "fid", "高可信候选C段:": "cidr24",
        "确认资产C段:": "cidr24", "种子C段:": "cidr24",
        "精确主机:": "seed_domain", "注册域:": "seed_domain",
        "证书域:": "seed_domain", "备案号:": "icp", "明确IP:": "seed_ip",
    }
    result = set()
    for piece in evidence.split(" | "):
        result.add(next((name for prefix, name in mapping.items() if piece.startswith(prefix)), "other"))
    return " | ".join(sorted(result))


def classify(row: dict[str, str], recent_since: str) -> tuple[str, str, str, str]:
    company = clean(row.get("company"))
    cert_org = clean(row.get("cert.subject.org"))
    evidence = clean(row.get("evidence"))
    protocol = clean(row.get("protocol")).lower()
    updated = clean(row.get("lastupdatetime"))[:10]
    status = clean(row.get("status_code"))

    cert_exact = bool(company and cert_org == company)
    name_match = any(marker in evidence for marker in NAME_MARKERS)
    # An explicitly supplied domain is already a strong scope assertion.  The
    # old classifier only trusted a certificate organisation equal to
    # `company`; domain-only projects use the domain itself as attribution
    # metadata, so every legitimate row was incorrectly sent to Q1 and the UI
    # lost P1/P2/P3 entirely.
    seed_domain_match = any(
        marker in evidence for marker in ("精确主机:", "注册域:", "证书域:")
    )
    strong_attribution = cert_exact or seed_domain_match
    is_web = "http" in protocol
    recent = updated >= recent_since
    active = status in ACTIVE_STATUSES

    attribution_reason = "给定域名范围命中" if seed_domain_match else "证书组织与企业全称一致"
    if strong_attribution and is_web and recent and active:
        return "P1_强归属且近期活跃", attribution_reason, "近期可响应", "priority"
    if strong_attribution and is_web and recent:
        return "P2_强归属但入口待验证", attribution_reason, f"HTTP状态:{status or '未知'}", "priority"
    if not strong_attribution and name_match and is_web and recent and active:
        return "P3_名称命中待确认", "页面标题/正文命中企业名称", "近期可响应", "priority"

    if not strong_attribution and not name_match:
        return "", "仅弱指纹/网络邻接，不能证明归属", "隔离", "Q1_弱证据隔离"
    if strong_attribution:
        return "", attribution_reason, "过旧或非Web", "Q2_强归属历史档案"
    return "", "页面名称命中", "过旧、非Web或无有效响应", "Q3_名称历史档案"


def new_cluster() -> dict[str, Any]:
    return {
        "assets": set(), "hosts": [], "domains": set(), "ips": set(),
        "status": Counter(), "tiers": Counter(), "latest": "", "subject_cn": set(),
    }


def update_cluster(cluster: dict[str, Any], row: dict[str, str], tier: str) -> None:
    asset = clean(row.get("asset_key"))
    host = clean(row.get("link")) or clean(row.get("host"))
    if asset:
        cluster["assets"].add(asset)
    if host and host not in cluster["hosts"] and len(cluster["hosts"]) < 5:
        cluster["hosts"].append(host)
    for field, target in [("domain", "domains"), ("ip", "ips"), ("cert.subject.cn", "subject_cn")]:
        value = clean(row.get(field))
        if value and len(cluster[target]) < 20:
            cluster[target].add(value)
    cluster["status"][clean(row.get("status_code")) or "unknown"] += 1
    cluster["tiers"][tier] += 1
    updated = clean(row.get("lastupdatetime"))
    if updated > cluster["latest"]:
        cluster["latest"] = updated


def write_clusters(path: Path, clusters: dict[tuple[str, str], dict[str, Any]], key_name: str) -> None:
    fields = [
        "company", key_name, "asset_count", "p1_count", "p2_count", "p3_count",
        "latest_update", "status_summary", "subject_cn_examples", "domain_examples",
        "ip_examples", "representative_urls", "recommendation",
    ]
    with path.open("w", encoding="utf-8-sig", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=fields)
        writer.writeheader()
        for (company, key), cluster in sorted(clusters.items(), key=lambda item: len(item[1]["assets"]), reverse=True):
            p1 = cluster["tiers"]["P1_强归属且近期活跃"]
            p2 = cluster["tiers"]["P2_强归属但入口待验证"]
            p3 = cluster["tiers"]["P3_名称命中待确认"]
            recommendation = "优先确认整个证书簇" if p1 else "抽查代表入口后决定"
            writer.writerow({
                "company": clean(company), key_name: clean(key),
                "asset_count": len(cluster["assets"]), "p1_count": p1,
                "p2_count": p2, "p3_count": p3,
                "latest_update": cluster["latest"],
                "status_summary": " | ".join(f"{k}:{v}" for k, v in cluster["status"].most_common()),
                "subject_cn_examples": " | ".join(sorted(cluster["subject_cn"])),
                "domain_examples": " | ".join(sorted(cluster["domains"])),
                "ip_examples": " | ".join(sorted(cluster["ips"])[:10]),
                "representative_urls": " | ".join(cluster["hosts"]),
                "recommendation": recommendation,
            })


def build_retry_plan(query_log: Path, output: Path) -> Counter:
    stats = Counter()
    if not query_log.exists():
        return stats
    with query_log.open(encoding="utf-8-sig", newline="") as source, output.open("w", encoding="utf-8-sig", newline="") as target:
        reader = csv.DictReader(source)
        fields = list(reader.fieldnames or []) + ["retry_priority", "retry_action", "recommended_query", "recommended_full", "recommended_interval"]
        writer = csv.DictWriter(target, fieldnames=fields)
        writer.writeheader()
        for row in reader:
            if clean(row.get("status")) == "ok":
                continue
            category = clean(row.get("category"))
            query = clean(row.get("query"))
            try:
                total = int(float(clean(row.get("total_matches")) or 0))
            except ValueError:
                total = 0

            if category in {"jarm", "header_hash", "banner_fid"}:
                priority, action, recommended = 9, "不重试：弱指纹全网重复度过高", ""
            elif category == "cidr24":
                priority, action = 3, "受控重试：只查Web协议；仍过宽时拆分/28"
                recommended = f'({query}) && (protocol="http" || protocol="https")'
            elif category in {"cert_org", "name_page", "seed_domain", "icp", "seed_ip"}:
                priority, action, recommended = 1, "优先重试种子查询", query
            elif category == "cert_serial" and total <= 10000:
                priority, action, recommended = 2, "重试证书查询，但只取近期数据", query
            elif category == "cert_serial":
                priority, action, recommended = 4, "证书结果过宽，先复核证书簇再决定", ""
            else:
                priority, action, recommended = 5, "人工判断", query
            stats[action] += 1
            output_row = dict(row)
            output_row.update({
                "retry_priority": priority, "retry_action": action,
                "recommended_query": recommended, "recommended_full": "false",
                "recommended_interval": "8.0",
            })
            writer.writerow({field: clean(output_row.get(field)) for field in fields})
    return stats


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="离线压缩 FOFA 候选为可人工复核的分层结果")
    parser.add_argument("--input", type=Path, default=BASE_DIR / "output" / "candidates.csv")
    parser.add_argument("--query-log", type=Path, default=BASE_DIR / "output" / "query_log.csv")
    parser.add_argument("--output-dir", type=Path, default=BASE_DIR / "refined_output_20260714")
    parser.add_argument("--recent-since", default="2025-01-01")
    parser.add_argument("--force", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not args.input.exists():
        print(f"[-] 输入不存在：{args.input}")
        return 2
    if args.output_dir.exists() and any(args.output_dir.iterdir()) and not args.force:
        print(f"[-] 输出目录已有内容：{args.output_dir}；如需覆盖新目录内容请添加 --force")
        return 2
    args.output_dir.mkdir(parents=True, exist_ok=True)

    try:
        csv.field_size_limit(sys.maxsize)
    except OverflowError:
        csv.field_size_limit(2**31 - 1)

    # 第一遍只计算进入复核集的资产被多少家企业同时命中。
    asset_companies: dict[str, set[str]] = defaultdict(set)
    bucket_counts = Counter()
    tier_counts = Counter()
    with args.input.open(encoding="utf-8-sig", newline="") as f:
        reader = csv.DictReader(f)
        source_fields = list(reader.fieldnames or [])
        for row in reader:
            tier, _, _, bucket = classify(row, args.recent_since)
            bucket_counts[bucket] += 1
            if tier:
                tier_counts[tier] += 1
                asset_companies[clean(row.get("asset_key"))].add(clean(row.get("company")))

    output_fields = DERIVED_FIELDS + source_fields
    tier_files = {
        "P1_强归属且近期活跃": args.output_dir / "P1_active_strong.csv",
        "P2_强归属但入口待验证": args.output_dir / "P2_strong_needs_validation.csv",
        "P3_名称命中待确认": args.output_dir / "P3_name_candidates.csv",
    }
    handles = {tier: path.open("w", encoding="utf-8-sig", newline="") for tier, path in tier_files.items()}
    combined_handle = (args.output_dir / "priority_review.csv").open("w", encoding="utf-8-sig", newline="")
    writers = {tier: csv.DictWriter(handle, fieldnames=output_fields) for tier, handle in handles.items()}
    combined_writer = csv.DictWriter(combined_handle, fieldnames=output_fields)
    for writer in [*writers.values(), combined_writer]:
        writer.writeheader()

    cert_clusters: dict[tuple[str, str], dict[str, Any]] = defaultdict(new_cluster)
    name_clusters: dict[tuple[str, str], dict[str, Any]] = defaultdict(new_cluster)
    try:
        with args.input.open(encoding="utf-8-sig", newline="") as f:
            for row in csv.DictReader(f):
                tier, reason, activity, _ = classify(row, args.recent_since)
                if not tier:
                    continue
                asset = clean(row.get("asset_key"))
                derived = {
                    "review_tier": tier,
                    "attribution_reason": reason,
                    "activity_state": activity,
                    "evidence_types": evidence_types(clean(row.get("evidence"))),
                    "company_match_count": len(asset_companies.get(asset, set())),
                }
                output_row = {**derived, **{field: clean(row.get(field)) for field in source_fields}}
                writers[tier].writerow(output_row)
                combined_writer.writerow(output_row)

                company = clean(row.get("company"))
                if tier.startswith("P1_") or tier.startswith("P2_"):
                    cert_key = clean(row.get("cert.sn")) or clean(row.get("cert.subject.cn")) or clean(row.get("domain")) or clean(row.get("ip"))
                    update_cluster(cert_clusters[(company, cert_key)], row, tier)
                else:
                    domain_key = clean(row.get("domain")) or clean(row.get("ip")) or clean(row.get("host"))
                    update_cluster(name_clusters[(company, domain_key)], row, tier)
    finally:
        combined_handle.close()
        for handle in handles.values():
            handle.close()

    write_clusters(args.output_dir / "certificate_clusters.csv", cert_clusters, "certificate_key")
    write_clusters(args.output_dir / "name_domain_clusters.csv", name_clusters, "domain_or_ip")
    retry_stats = build_retry_plan(args.query_log, args.output_dir / "retry_plan.csv")

    summary = {
        "source": str(args.input), "source_rows": sum(bucket_counts.values()),
        "recent_since": args.recent_since, "tier_counts": dict(tier_counts),
        "bucket_counts": dict(bucket_counts), "priority_rows": sum(tier_counts.values()),
        "certificate_clusters": len(cert_clusters), "name_domain_clusters": len(name_clusters),
        "retry_actions": dict(retry_stats),
        "notes": [
            "原 output 未修改；弱指纹明细继续保留在原 candidates.csv。",
            "P1/P2 表示归属证据强，不等同于入口一定可以在浏览器直接打开。",
            "retry_plan 只是离线计划，不会自动请求 FOFA。",
        ],
    }
    (args.output_dir / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    with (args.output_dir / "summary.csv").open("w", encoding="utf-8-sig", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["metric", "value"])
        writer.writerow(["source_rows", summary["source_rows"]])
        writer.writerow(["priority_rows", summary["priority_rows"]])
        for key, value in tier_counts.items():
            writer.writerow([key, value])
        for key, value in bucket_counts.items():
            writer.writerow([key, value])
        writer.writerow(["certificate_clusters", len(cert_clusters)])
        writer.writerow(["name_domain_clusters", len(name_clusters)])

    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
