#!/usr/bin/env python3
"""使用官方 Hacker API 同步项目、Scope 与排除项到本地 SQLite。"""
from __future__ import annotations

import argparse, base64, hashlib, json, os, sqlite3, urllib.request
from datetime import datetime
from urllib.parse import urljoin

BASE="https://api.hackerone.com/v1"

def request(path):
    user=os.environ.get("H1_API_USERNAME",""); token=os.environ.get("H1_API_TOKEN","")
    if not user or not token: raise RuntimeError("请配置 HackerOne API Token identifier 和 token")
    auth=base64.b64encode(f"{user}:{token}".encode()).decode()
    req=urllib.request.Request(path if path.startswith("http") else BASE+path,headers={"Accept":"application/json","Authorization":f"Basic {auth}","User-Agent":"oviraptor/0.3"})
    with urllib.request.urlopen(req,timeout=45) as response: return json.load(response)

def pages(path):
    url=BASE+path; result=[]
    while url:
        data=request(url); result.extend(data.get("data",[])); url=(data.get("links") or {}).get("next")
    return result

def val(v): return 1 if v else 0
def now(): return datetime.now().astimezone().isoformat(timespec="seconds")

def save_program(db,item):
    a=item.get("attributes") or {}; handle=a.get("handle") or item.get("id"); policy=a.get("policy") or ""; digest=hashlib.sha256(policy.encode()).hexdigest()
    old=db.execute("select policy_hash,submission_state from hackerone_programs where handle=?",(handle,)).fetchone()
    db.execute("""insert into hackerone_programs(id,handle,name,icon_url,policy,policy_hash,submission_state,program_state,offers_bounties,open_scope,fast_payments,safe_harbor,collaboration,started_accepting_at,last_synced_at)
      values(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) on conflict(handle) do update set id=excluded.id,name=excluded.name,icon_url=excluded.icon_url,policy=excluded.policy,policy_hash=excluded.policy_hash,submission_state=excluded.submission_state,program_state=excluded.program_state,offers_bounties=excluded.offers_bounties,open_scope=excluded.open_scope,fast_payments=excluded.fast_payments,safe_harbor=excluded.safe_harbor,collaboration=excluded.collaboration,started_accepting_at=excluded.started_accepting_at,last_synced_at=excluded.last_synced_at""",
      (str(item.get("id")),handle,a.get("name") or handle,urljoin("https://hackerone.com",a.get("profile_picture") or ""),policy,digest,a.get("submission_state") or "",a.get("state") or "",val(a.get("offers_bounties")),val(a.get("open_scope")),val(a.get("fast_payments")),val(a.get("gold_standard_safe_harbor")),val(a.get("allows_bounty_splitting")),a.get("started_accepting_at"),now()))
    if old and old[0] != digest: db.execute("insert into hackerone_events(program_handle,event_type,summary) values(?,'policy_changed','Policy 内容发生变化')",(handle,))
    if old and old[1] != (a.get("submission_state") or ""): db.execute("insert into hackerone_events(program_handle,event_type,summary) values(?,'submission_state_changed',?)",(handle,f"提交状态：{old[1]} → {a.get('submission_state') or ''}"))
    return handle

def sync_list(db):
    items=pages("/hackers/programs?page[size]=100")
    for item in items: save_program(db,item)
    return {"programs":len(items)}

def sync_detail(db,handle):
    program=request(f"/hackers/programs/{handle}").get("data") or {}; save_program(db,program)
    scopes=pages(f"/hackers/programs/{handle}/structured_scopes?page[size]=100")
    previous={r[0]:(r[1],r[2],r[3],r[4]) for r in db.execute("select id,eligible_for_submission,eligible_for_bounty,max_severity,instruction from hackerone_scopes where program_handle=? and active=1",(handle,))}
    seen=set()
    for item in scopes:
        a=item.get("attributes") or {}; sid=str(item.get("id")); seen.add(sid); current=(val(a.get("eligible_for_submission")),val(a.get("eligible_for_bounty")),a.get("max_severity") or "",a.get("instruction") or "")
        db.execute("""insert into hackerone_scopes(id,program_handle,asset_type,asset_identifier,eligible_for_submission,eligible_for_bounty,max_severity,instruction,reference,created_at,updated_at,active) values(?,?,?,?,?,?,?,?,?,?,?,1)
          on conflict(id) do update set program_handle=excluded.program_handle,asset_type=excluded.asset_type,asset_identifier=excluded.asset_identifier,eligible_for_submission=excluded.eligible_for_submission,eligible_for_bounty=excluded.eligible_for_bounty,max_severity=excluded.max_severity,instruction=excluded.instruction,reference=excluded.reference,updated_at=excluded.updated_at,active=1""",
          (sid,handle,a.get("asset_type") or "",a.get("asset_identifier") or "",*current,a.get("reference") or "",a.get("created_at"),a.get("updated_at")))
        if sid not in previous: db.execute("insert into hackerone_events(program_handle,event_type,summary) values(?,'scope_added',?)",(handle,f"新增 Scope：{a.get('asset_identifier') or ''}"))
        elif previous[sid] != current: db.execute("insert into hackerone_events(program_handle,event_type,summary) values(?,'scope_changed',?)",(handle,f"Scope 变化：{a.get('asset_identifier') or ''}"))
    for sid in set(previous)-seen:
        db.execute("update hackerone_scopes set active=0 where id=?",(sid,)); db.execute("insert into hackerone_events(program_handle,event_type,summary) values(?,'scope_removed',?)",(handle,f"移除 Scope：{sid}"))
    exclusions=pages(f"/hackers/programs/{handle}/scope_exclusions?page[size]=100")
    db.execute("update hackerone_exclusions set active=0 where program_handle=?",(handle,))
    for item in exclusions:
        a=item.get("attributes") or {}; db.execute("""insert into hackerone_exclusions(id,program_handle,category,details,updated_at,active) values(?,?,?,?,?,1) on conflict(id) do update set category=excluded.category,details=excluded.details,updated_at=excluded.updated_at,active=1""",(str(item.get("id")),handle,a.get("category") or "",a.get("details") or "",a.get("updated_at")))
    return {"handle":handle,"scopes":len(scopes),"exclusions":len(exclusions)}

def main():
    p=argparse.ArgumentParser(); p.add_argument("--db",required=True); p.add_argument("--handle"); a=p.parse_args()
    db=sqlite3.connect(a.db,timeout=60)
    try:
        result=sync_detail(db,a.handle) if a.handle else sync_list(db); db.commit(); print(json.dumps(result,ensure_ascii=False)); return 0
    finally: db.close()

if __name__=="__main__": raise SystemExit(main())
