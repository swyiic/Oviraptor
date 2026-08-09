#!/usr/bin/env python3
"""压缩 SRC Web 探测结果；保留原文件，输出优选与可恢复隔离清单。"""
from __future__ import annotations

import argparse, csv, ipaddress, json
from pathlib import Path
from urllib.parse import urlsplit


def text(row, *keys):
    for key in keys:
        value = str(row.get(key) or "").strip()
        if value:
            return value
    return ""


def endpoint(row):
    raw = text(row, "probe_effective_url", "link", "host")
    parsed = urlsplit(raw if "://" in raw else f"//{raw}")
    host = (parsed.hostname or text(row, "domain", "ip")).lower().rstrip(".")
    if host.startswith("www."):
        host = host[4:]
    path = (parsed.path or "/").rstrip("/") or "/"
    return f"{text(row, 'company').lower()}|{host}|{path}"


def rank(row):
    try: status = int(text(row, "probe_status_code", "status_code") or 0)
    except ValueError: status = 0
    outcome = text(row, "probe_outcome")
    url = text(row, "probe_effective_url", "link", "host")
    status_rank = 5 if 200 <= status < 300 else 4 if 300 <= status < 400 else 3 if status in {401,403} else 2 if 400 <= status < 500 else 0
    outcome_rank = {
        "web_alive": 6, "virtual_host_required": 5,
        "web_restricted": 4, "browser_render_required": 3,
        "tcp_alive_non_http": 2, "alive_clean": 1,
    }.get(outcome, 0)
    return (outcome_rank, status_rank, url.startswith("https://"), bool(text(row,"probe_title","title")))


def reason(row):
    try: status = int(text(row, "probe_status_code", "status_code") or 0)
    except ValueError: status = 0
    outcome = text(row, "probe_outcome")
    if outcome == "web_abnormal":
        state = text(row, "probe_entry_state") or f"HTTP_{status}"
        return f"web_abnormal:{state}"
    if outcome == "unreachable":
        state = text(row, "probe_entry_state") or "connect_failed"
        return f"unreachable:{state}"
    if outcome == "skipped":
        return f"skipped:{text(row, 'probe_error') or 'invalid_target'}"
    return ""


def write(path, fields, rows):
    with path.open("w", encoding="utf-8-sig", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=fields, extrasaction="ignore")
        writer.writeheader(); writer.writerows(rows)


def main():
    p=argparse.ArgumentParser(); p.add_argument("--input-dir",type=Path,required=True); p.add_argument("--output-dir",type=Path,required=True); a=p.parse_args()
    a.output_dir.mkdir(parents=True,exist_ok=True)
    rows=[]; fields=[]
    for path in sorted(a.input_dir.glob("*.csv")):
        if not any(x in path.name for x in (
            "web_alive", "web_restricted", "browser_render_required",
            "virtual_host_required", "web_abnormal", "tcp_alive_non_http",
            "alive_clean", "blocked_content", "unreachable", "skipped",
        )): continue
        with path.open(encoding="utf-8-sig",newline="") as f:
            reader=csv.DictReader(f); fields=list(dict.fromkeys(fields+list(reader.fieldnames or [])))
            rows.extend(dict(row) for row in reader)
    best={}
    for row in rows:
        if reason(row): continue
        key=endpoint(row)
        if key not in best or rank(row)>rank(best[key]): best[key]=row
    keep=[]; excluded=[]
    for row in rows:
        why=reason(row)
        if not why and best.get(endpoint(row)) is not row: why="duplicate_endpoint"
        out=dict(row)
        if why:
            out.update({"auto_excluded":"1","exclude_reason":why}); excluded.append(out)
        else: keep.append(out)
    fields=list(dict.fromkeys(fields+["auto_excluded","exclude_reason"]))
    write(a.output_dir/"optimized_assets.csv",fields,keep); write(a.output_dir/"auto_excluded.csv",fields,excluded)
    summary={
        "input":len(rows),"kept":len(keep),"excluded":len(excluded),
        "web_alive":sum(text(r,"probe_outcome") in {"web_alive","web_restricted","browser_render_required","virtual_host_required"} for r in rows),
        "tcp_non_web":sum(text(r,"probe_outcome")=="tcp_alive_non_http" for r in rows),
        "web_abnormal":sum(text(r,"probe_outcome")=="web_abnormal" for r in rows),
        "unreachable":sum(reason(r).startswith("unreachable:") for r in rows),
    }
    (a.output_dir/"summary.json").write_text(json.dumps(summary,ensure_ascii=False,indent=2),encoding="utf-8")
    print(json.dumps(summary,ensure_ascii=False))
    return 0

if __name__=="__main__": raise SystemExit(main())
