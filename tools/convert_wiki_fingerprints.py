#!/usr/bin/env python3
"""Offline, allowlisted Wiki identification conversion; never replay POCs or open Ovi DBs.

Produces current ANY-only loader-compatible JSON, an independently checksummed pack,
and a row-complete provenance/rejection ledger. Originals and existing rules stay intact.
"""
from __future__ import annotations
import argparse
import collections
import csv
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import tempfile
import unicodedata

from wiki_fingerprint_dsl import (ConversionError, convert_query, convert_query_tree, normalize_query,
                                  query_predicates, query_predicate_tree, query_predicates_tree)

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / 'tools/run_fingerprint_pack_cli.py'
GENERIC = {'login','admin','index','index.html','/login','/login.html','/index.html','home','welcome','terminal','system','username','password','jquery','登录','用户登录','系统登录','后台登录','管理系统','公告','帮助中心','帮助文档','api docs','swagger ui','404','404 not found','tutorial','error querying database.','error: username or password error,please input again.','not found','http status 404 – not found','http status 404 - not found'}
VAGUE_PRODUCTS = {'windows','linux','unix','unknown','none','null','未知','其他','未命名','http','https','web'}
SECRET = re.compile(r"(?i)(?:jsessionid|phpsessid|session(?:id)?|token|password|passwd|secret(?:[_-]?key)?|access[_-]?key(?:[_-]?id)?|api[_-]?key|authorization)\s*[\"']?\s*[:=]\s*[\"']?\s*[a-z0-9+/=._-]{12,}|eyJ[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}")
PAYLOAD = re.compile(r'(?i)\bCVE-\d{4}-\d+\b|\b(?:whoami|xp_cmdshell|sleep\s*\(|union\s+select|runtime\.getruntime|os\.system|/etc/passwd|uid=0)\b')
BAD_PRODUCT = re.compile(r'(?i)CVE[-_ ]?\d{4}|\b(?:rce|sqli|xss)\b|漏洞|未授权|任意文件|命令执行|代码执行|注入漏洞|信息泄露|[<>]=')
GENERIC_FIXTURES = [
 ('<html><head><title>Login</title></head><body>login admin username password /login /static/js/jquery.min.js</body></html>', {'server':'nginx','content-type':'text/html'}),
 ('<html><head><title>系统登录</title></head><body>登录 用户名 密码 管理系统</body></html>', {'server':'Apache','content-type':'text/html'}),
 ('<html><head><title>公告</title></head><body>正常公告 帮助中心 帮助文档 API docs jQuery</body></html>', {'content-type':'text/html'}),
 ('<html><head><title>404 Not Found</title></head><body>Not Found</body></html>', {'server':'nginx'}),
]

def sha(path: Path) -> str:
    h=hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda:f.read(1024*1024),b''): h.update(block)
    return h.hexdigest()

def compact(value): return json.dumps(value,ensure_ascii=False,sort_keys=True,separators=(',',':'))
def product_key(name): return ''.join(c for c in unicodedata.normalize('NFKC',name).casefold() if c.isalnum())
def safe_name(names):
    values={product_key(n):n.strip() for n in names if isinstance(n,str) and n.strip()}
    if len(values)!=1: return None
    key,name=next(iter(values.items()))
    if key in VAGUE_PRODUCTS or BAD_PRODUCT.search(name) or re.search(r'\b(?:body|title|server|header|app)\s*=',name,re.I) or len(name.encode())>256: return None
    return name

def collect_sources(knowledge: Path, history: Path):
    records=[]; inventory=[]; ignored=[]
    for path in sorted(history.rglob('*.json')):
        if path.is_symlink(): continue
        if path.stat().st_size>64*1024*1024:
            ignored.append({'path':str(path),'reason':'source_file_over_64MiB'});continue
        try: doc=json.loads(path.read_text(encoding='utf-8-sig'))
        except (UnicodeError,json.JSONDecodeError):
            ignored.append({'path':str(path),'reason':'not_readable_JSON'});continue
        rows=doc if isinstance(doc,list) else [doc]
        chosen=[(i,x) for i,x in enumerate(rows) if isinstance(x,dict) and isinstance(x.get('Product'),str) and isinstance(x.get('FofaQuery',x.get('GobyQuery')),str)]
        if not chosen: continue
        digest=sha(path);inventory.append({'path':str(path),'sha256':digest,'rows':len(chosen),'kind':'goby_identification_fields'})
        for index,item in chosen:
            key='FofaQuery' if isinstance(item.get('FofaQuery'),str) else 'GobyQuery'
            records.append({'record_id':f'goby:{digest}:{index}:{key}','source_kind':'goby','source_path':str(path),'source_pointer':f'/{index}/{key}' if isinstance(doc,list) else f'/{key}', 'source_sha256':digest,'products':[item['Product']], 'query':item[key],'eligible':True})
    db=(knowledge/'features/catalog.sqlite').resolve()
    inventory.append({'path':str(db),'sha256':sha(db),'kind':'knowledge_features_readonly'})
    con=sqlite3.connect(db.as_uri()+'?mode=ro&immutable=1',uri=True);con.row_factory=sqlite3.Row
    try:
        evidence=collections.defaultdict(list)
        for row in con.execute("SELECT e.feature_id,e.derivation,e.location_json,e.confidence,s.kind,s.representative_path,s.sha256 FROM evidence e JOIN features f ON f.feature_id=e.feature_id JOIN source_objects s ON s.sha256=e.source_sha256 WHERE f.kind='product_fingerprint'"):
            evidence[row['feature_id']].append(dict(row))
        for row in con.execute("SELECT feature_id,payload_json,products_json,confidence FROM features WHERE kind='product_fingerprint' ORDER BY feature_id"):
            payload=json.loads(row['payload_json']);ev=evidence[row['feature_id']]
            non_image=[e for e in ev if e['kind']!='image' and e['confidence']>=0.75]
            representative=(non_image or ev or [{}])[0]
            records.append({'record_id':row['feature_id'],'source_kind':'feature_dsl' if 'query' in payload else 'header_observation','source_path':representative.get('representative_path',''),'source_pointer':representative.get('location_json',''),'source_sha256':representative.get('sha256',''),'products':json.loads(row['products_json']),'query':payload.get('query'),'eligible':bool(non_image) and row['confidence']>=0.75})
        exclusions=dict(con.execute("SELECT kind,count(*) FROM features WHERE kind IN ('product_identifier','product_hint','header_literal') GROUP BY kind"))
    finally: con.close()
    return records,inventory,ignored,exclusions

def canonical_condition(node):
    if node['type'] in ('all','any'):
        children=[]
        for child in node['children']:
            child=canonical_condition(child)
            children.extend(child['children'] if child['type']==node['type'] else [child])
        unique={compact(child):child for child in children}
        children=[unique[key] for key in sorted(unique)]
        return children[0] if len(children)==1 else {'type':node['type'],'children':children}
    return dict(node)


def canonical_fields(fields):
    if 'condition' in fields:
        return {'condition':canonical_condition(fields['condition'])}
    out={}
    for name in ['headerRegex','setCookieRegex','bodyRegex','pathSignals']:
        if not fields.get(name): continue
        if name=='headerRegex': out[name]={k.lower():v for k,v in sorted(fields[name].items())}
        elif name in ('setCookieRegex','bodyRegex'): out[name]=sorted(set(fields[name]))
        else: out[name]=fields[name]
    return out

def weak_literal(value):
    text=unicodedata.normalize('NFKC',value).strip().casefold()
    if text in GENERIC: return True
    if re.match(r'^(?:http\s+status\s+)?(?:400|401|403|404|405|408|410|413|429|500|502|503|504)(?:\b|[\s:–-])', text): return True
    if text in {'authorization is required','authorization required','unauthorized','forbidden','access denied','web interface','web management','device management','index of /'}: return True
    letters=[c for c in text if c.isalnum()]
    return len(letters)<(2 if any(ord(c)>127 for c in letters) else 4)

def matches_generic(fields):
    for body,headers in GENERIC_FIXTURES:
        signals=[bool(re.search(pattern,body)) for pattern in fields.get('bodyRegex',[])]
        signals += [bool(re.search(pattern,headers.get(name.lower(),''))) for name,pattern in fields.get('headerRegex',{}).items()]
        if signals and (any(signals) if fields.get('matchMode','any')=='any' else all(signals)): return True
    return False

def convert_records(records, builtins):
    builtin_ids={rule['id'] for rule in builtins}
    protected={product_key(n):rule['id'] for rule in builtins for n in [rule['id'],rule['name']]}
    builtin_fields={compact(canonical_fields(rule)):rule['id'] for rule in builtins}
    ledger=[];groups=collections.defaultdict(list)
    for record in records:
        query=record.get('query')
        item={k:v for k,v in record.items() if k not in ('query','eligible')}
        item['query_sha256']=hashlib.sha256((query or '').encode()).hexdigest()
        item['status']='rejected';item['rule_id']=''
        ledger.append(item)
        def reject(code):item['reason']=code
        if record['source_kind']=='header_observation':reject('response_header_observation_not_product_rule');continue
        if not record['eligible']:reject('ocr_only_or_low_confidence_evidence');continue
        try:
            fields=convert_query(query);predicates=query_predicates(query)
            item['normalized_query_sha256']=hashlib.sha256(normalize_query(query).encode()).hexdigest()
        except ConversionError as e:reject('dsl_'+e.code);continue
        # Current scanning/exposure_probe.rs ignores matchMode; never activate an
        # AND by letting that loader silently treat its signals as OR.
        if fields.get('matchMode','any')!='any':reject('current_loader_has_no_AND_support');continue
        if any(SECRET.search(p['value']) for p in predicates):reject('historical_credential_or_session_literal');continue
        if any(PAYLOAD.search(p['value']) for p in predicates):reject('vulnerability_or_exploit_literal_not_identity');continue
        if any(weak_literal(p['value']) for p in predicates) or matches_generic(fields):reject('generic_or_non_product_specific_marker');continue
        # A shared frontend dependency is not evidence for an unrelated app.
        def shared_dependency(predicate):
            value=predicate['value'];lower=value.casefold()
            shared=any(part in lower for part in ('jquery','bootstrap','layui','element-ui','vue.min.js','vue.js','react.min.js','angular.min.js','artdialog','sea.js','seaconfig.js','favicon.ico','loginstyle.css','base.css'))
            related=any(product_key(n) and product_key(n) in product_key(value) for n in record['products'])
            return shared and not related
        if any(shared_dependency(p) for p in predicates):reject('shared_dependency_not_product_identity');continue
        # A generic HTTP server is evidence for that server component, not an
        # unrelated application running behind it.
        unrelated_server=False; generic_header=False
        generic_headers={'cache-control','etag','date','expires','last-modified','content-type','content-length','connection','keep-alive','accept-ranges','transfer-encoding','vary','pragma','strict-transport-security','content-security-policy','x-frame-options','x-content-type-options','access-control-allow-origin'}
        for predicate in predicates:
            header_name='';header_value=''
            if predicate['field']=='server':header_name,header_value='server',predicate['value']
            elif predicate['field']=='header':
                name,_,content=predicate['value'].partition(':');header_name,header_value=name.casefold(),content.lstrip(' \t')
            if header_name in generic_headers:generic_header=True
            if header_name=='server':
                value_key=product_key(header_value)
                related=any(product_key(n) in value_key or (len(value_key)>=4 and value_key in product_key(n)) for n in record['products'] if product_key(n))
                if not related:unrelated_server=True
        if generic_header:reject('generic_response_header_not_product_identity');continue
        if unrelated_server:reject('server_component_not_application_identity');continue
        fields=canonical_fields(fields)
        key=compact(fields)
        if key in builtin_fields:
            item.update(status='protected_existing',reason='existing_builtin_signature',rule_id=builtin_fields[key]);continue
        primary=record['source_kind']=='goby'
        name=safe_name(record['products'])
        if name and product_key(name) in protected:
            item.update(status='protected_existing',reason='existing_builtin_product_preserved',rule_id=protected[product_key(name)]);continue
        groups[key].append((item,fields,name,primary))
    rules=[]
    for key,group in sorted(groups.items()):
        primary_names={product_key(name):name for _,_,name,primary in group if primary and name}
        secondary_names={product_key(name):name for _,_,name,_ in group if name}
        names=primary_names or secondary_names
        if len(names)!=1:
            for item,_,_,_ in group:item['reason']='missing_product_context' if not names else 'ambiguous_products_for_identical_signal'
            continue
        name=next(iter(names.values()))
        if not primary_names:
            # Derived article-wide labels can describe related products or a
            # sidebar. Require an explicit literal link before naming the app.
            related=False
            for item,_,_,_ in group:
                record=next(r for r in records if r['record_id']==item['record_id'])
                if any(product_key(name) in product_key(p['value']) for p in query_predicates(record['query'])):
                    related=True;break
            if not related:
                for item,_,_,_ in group:item['reason']='unconfirmed_product_association'
                continue
        rule_id='wiki-'+hashlib.sha256((product_key(name)+'\n'+key).encode()).hexdigest()[:24]
        assert rule_id not in builtin_ids
        reviewed_bad={
            'wiki-9d9d8a5c4977bfcc3c566f3b':'reviewed_KubeOperator_query_identifies_KubePi',
            'wiki-a15ba0c9efb9224164a46afa':'reviewed_Brother_query_identifies_Rainwise',
            'wiki-b2499c33e91abf74ac6157d2':'reviewed_query_text_is_not_product_name',
            'wiki-2b75744260a6c62343dfe5df':'reviewed_generic_client_list_and_shared_JS',
        }
        if rule_id in reviewed_bad:
            for item,_,_,_ in group:item['reason']=reviewed_bad[rule_id]
            continue
        fields=group[0][1]
        rules.append({'id':rule_id,'name':name,**fields})
        representative=next((item for item,_,n,primary in group if primary and n),group[0][0])
        for item,_,_,_ in group:
            item.update(status='converted' if item is representative else 'duplicate',reason='converted_current_ANY_format' if item is representative else 'same_canonical_identification_conditions',rule_id=rule_id)
    return sorted(rules,key=lambda x:x['id']),ledger


# A weak predicate may restrict a branded conjunction, but cannot independently
# identify an application. Sensitivity and unsupported syntax are hard failures.
GENERIC_HEADERS={'cache-control','etag','date','expires','last-modified','content-type','content-length','connection','keep-alive','accept-ranges','transfer-encoding','vary','pragma','strict-transport-security','content-security-policy','x-frame-options','x-content-type-options','access-control-allow-origin'}
SHARED_DEPENDENCIES=('jquery','bootstrap','layui','element-ui','vue.min.js','vue.js','react.min.js','angular.min.js','artdialog','sea.js','seaconfig.js','favicon.ico','loginstyle.css','base.css')



GENERIC_BRAND_WORDS={
    'application','apps','app','server','system','platform','software','management','enterprise','console',
    'network','camera','router','firewall','series','printer','monitor','dashboard','admin','portal','center',
    'device','web','cloud','smart','security','connect','service','services','login','index','default','public',
    'content','member','control','controller','interface','http','https','pro','plus','classic','community',
    '系统','管理系统','平台','管理平台','软件','摄像头','路由器','打印机','监控','中心','物联网','信息',
    '协同办公','企业管理','用户登录','登录','企业信息','网络','设备','无线','智能','防火墙','中文版',
    '日志系统','档案局','广播系统','控制系统','摄像机','交换机','信息系统','资讯系统','报表系统',
    '门禁系统','认证系统','应用系统','办公系统','视频系统','内容管理','文件管理','运维平台','网络设备',
    'video','client','networkvideoclient','gpon','gponrouter','link','dir',
}

VENDOR_BRANDS={'apache','oracle','microsoft','google','amazon','ibm','cisco','vmware','netgear','dlink','hikvision','zte','xerox','fujitsu'}
GENERIC_BRAND_WORDS.update(VENDOR_BRANDS)


def literal_links_product(value,products):
    """Require explicit identity text, not mere presence of a long pathname.

    This deliberately leaves opaque vendor abbreviations and unbranded asset
    combinations for manual review rather than assigning them a guessed brand.
    """
    value_key=product_key(value)
    for name in products:
        normalized=unicodedata.normalize('NFKC',name)
        whole=product_key(normalized)
        if whole and (whole not in GENERIC_BRAND_WORDS or whole in VENDOR_BRANDS) and whole in value_key:
            return True
        latin=re.findall(r'[A-Za-z][A-Za-z0-9]{3,}',normalized)
        chinese=re.findall(r'[\u4e00-\u9fff]{2,}',normalized)
        models=re.findall(r'[A-Za-z][A-Za-z0-9]*(?:[-_.][A-Za-z0-9]+)+',normalized)
        for token in [*latin,*chinese,*models]:
            key=product_key(token)
            if key and key not in GENERIC_BRAND_WORDS and key in value_key:
                return True
    return False


def predicate_strength(predicate, products):
    """Return (strong_enough_to_stand_alone, reason) for one source literal.

    Strong means it survives conservative local specificity gates, not that the
    historical source has been independently verified against a live product.
    """
    value=predicate['value'];field=predicate['field'];value_key=product_key(value)
    related=any(product_key(name) and product_key(name) in value_key for name in products)
    if weak_literal(value):return False,'generic_or_non_product_specific_marker'
    if any(part in value.casefold() for part in SHARED_DEPENDENCIES) and not related:
        return False,'shared_dependency_not_product_identity'
    if not related and (re.search(r'(?i)validatecode|captcha|txt(?:user(?:name)?|pass(?:word)?|yzm)|password error|username error',value)
                        or re.search(r'(?i)(?:class|id)=[\"\']?[^<>]*(?:height[0-9]|lineheight|clearfix|client-list|dm-clear)',value)):
        return False,'generic_authentication_or_layout_marker'
    header_name='';header_value=''
    if field=='server':header_name,header_value='server',value
    elif field=='header':
        name,_,content=value.partition(':');header_name,header_value=name.casefold(),content.lstrip(' \t')
    if header_name in GENERIC_HEADERS:return False,'generic_response_header_not_product_identity'
    if header_name:
        value_key=product_key(header_value);name_key=product_key(header_name)
        if not any(product_key(name) in value_key or product_key(name) in name_key
                   or (len(value_key)>=4 and value_key in product_key(name))
                   for name in products if product_key(name)):
            return False,'server_or_header_component_not_application_identity'
    if not literal_links_product(value,products):return False,'unconfirmed_unbranded_marker'
    return True,'explicit_product_marker'


def expression_strength(expression,products):
    """Whether every satisfying branch necessarily contains specific evidence.

    ALL needs at least one mandatory strong child; ANY needs every alternative
    to be strong. For example Brand && (Login || 404) remains brand-gated, while
    Brand || Login does not. No child is removed from the expression.
    """
    if expression['type']=='literal':
        strong,reason=predicate_strength(expression,products)
        return strong,([] if strong else [reason])
    children=[expression_strength(child,products) for child in expression['children']]
    if not children:return False,['empty_boolean_group']
    strong=any(item[0] for item in children) if expression['type']=='all' else all(item[0] for item in children)
    return strong,sorted({reason for _,reasons in children for reason in reasons})


def evaluate_condition(condition,body='',headers=None):
    """Offline fixture evaluator; production Rust is the final format checker."""
    headers={name.lower():value for name,value in (headers or {}).items()}
    kind=condition['type']
    if kind in ('all','any'):
        values=[evaluate_condition(child,body,headers) for child in condition['children']]
        return bool(values) and (all(values) if kind=='all' else any(values))
    if kind=='body':return bool(re.search(condition['pattern'],body))
    if kind=='header':return bool(re.search(condition['pattern'],headers.get(condition['name'].lower(),'')))
    raise ValueError('Only passive body/header source predicates are converted')



MAX_COVERAGE_TERMS=1024


def exact_condition_atom(node):
    """Only field identity and byte-identical regexes participate in proofs."""
    kind=node['type']
    if kind in ('body','cookie'):return (kind,node['pattern'])
    if kind=='header':return (kind,node['name'].lower(),node['pattern'])
    # Do not infer equivalence for paths or future leaf kinds.
    return ('unsupported',compact(node))


def minimal_positive_dnf(condition,limit=MAX_COVERAGE_TERMS):
    """Bounded exact positive DNF; None means unknown, never redundant.

    Superset terms can be absorbed by a subset term in a positive expression.
    The bound is checked before cross products so a hostile tree cannot make
    conversion spend unbounded work proving an optional deduplication.
    """
    def minimize(terms):
        unique=set(terms)
        if len(unique)>limit:return None
        result=[]
        for term in sorted(unique,key=lambda item:(len(item),repr(sorted(item)))):
            if not any(previous<=term for previous in result):result.append(term)
        return result
    kind=condition['type']
    if kind not in ('all','any'):return [frozenset((exact_condition_atom(condition),))]
    children=condition['children']
    if not children:return None
    terms=[frozenset()] if kind=='all' else []
    for child in children:
        right=minimal_positive_dnf(child,limit)
        if right is None:return None
        if kind=='all':
            if len(terms)*len(right)>limit:return None
            terms=minimize([left|other for left in terms for other in right])
        else:
            if len(terms)+len(right)>limit:return None
            terms=minimize([*terms,*right])
        if terms is None:return None
    return terms


def legacy_any_atoms(rule):
    if rule.get('condition') is not None or rule.get('matchMode','any')!='any':return frozenset()
    atoms={('body',pattern) for pattern in rule.get('bodyRegex',[])}
    atoms.update(('cookie',pattern) for pattern in rule.get('setCookieRegex',[]))
    atoms.update(('header',name.lower(),pattern) for name,pattern in rule.get('headerRegex',{}).items())
    return frozenset(atoms)


def existing_rule_coverage(name,condition,existing_rules):
    """Return proving IDs only when same-name old Any rules cover EVERY term."""
    same=[]
    for rule in existing_rules:
        if product_key(rule['name'])!=product_key(name):continue
        atoms=legacy_any_atoms(rule)
        if atoms:same.append((rule['id'],atoms))
    if not same:return []
    terms=minimal_positive_dnf(condition)
    if not terms:return []
    covered_by=set()
    for term in terms:
        hits={identifier for identifier,atoms in same if term&atoms}
        if not hits:return []
        covered_by.update(hits)
    return sorted(covered_by)


def convert_boolean_records(records,builtins,existing_rules=()):
    """Incremental tree rules only; original Any output and IDs stay untouched."""
    builtin_ids={rule['id'] for rule in builtins}
    protected={product_key(name):rule['id'] for rule in builtins for name in (rule['id'],rule['name'])}
    existing_signatures={compact(canonical_fields(rule)):rule['id'] for rule in [*builtins,*existing_rules]}
    ledger=[];groups=collections.defaultdict(list);by_record={r['record_id']:r for r in records}
    for record in records:
        query=record.get('query');item={k:v for k,v in record.items() if k not in ('query','eligible')}
        item.update(query_sha256=hashlib.sha256((query or '').encode()).hexdigest(),status='rejected',rule_id='')
        ledger.append(item)
        if record['source_kind']=='header_observation':item['reason']='response_header_observation_not_product_rule';continue
        if not record['eligible']:item['reason']='ocr_only_or_low_confidence_evidence';continue
        try:
            fields=convert_query_tree(query)
            expression=query_predicate_tree(query);predicates=query_predicates_tree(query)
            item['normalized_query_sha256']=hashlib.sha256(normalize_query(query).encode()).hexdigest()
        except ConversionError as error:item['reason']='dsl_'+error.code;continue
        if 'condition' not in fields:
            item.update(status='not_incremental',reason='legacy_ANY_serialization_preserved');continue
        if any(SECRET.search(p['value']) for p in predicates):item['reason']='historical_credential_or_session_literal';continue
        if any(PAYLOAD.search(p['value']) for p in predicates):item['reason']='vulnerability_or_exploit_literal_not_identity';continue
        strong,weak_reasons=expression_strength(expression,record['products'])
        item['supportive_conditions']=weak_reasons
        if not strong:item['reason']='boolean_branch_without_specific_evidence';continue
        if any(evaluate_condition(fields['condition'],body,headers) for body,headers in GENERIC_FIXTURES):
            item['reason']='condition_matches_generic_fixture';continue
        fields=canonical_fields(fields);key=compact(fields)
        if key in existing_signatures:
            item.update(status='protected_existing',reason='existing_condition_signature',rule_id=existing_signatures[key]);continue
        primary=record['source_kind']=='goby';name=safe_name(record['products'])
        if name and product_key(name) in protected:
            item.update(status='protected_existing',reason='existing_builtin_product_preserved',rule_id=protected[product_key(name)]);continue
        # Explicit wrong-product pairings seen during source review remain
        # quarantined even if additional predicates wrap the bad title.
        labels={product_key(n) for n in record['products']};literal_keys=[product_key(p['value']) for p in predicates]
        if ('kubeoperator' in labels and any('kubepi' in value for value in literal_keys)
                or any('brother' in name for name in labels) and any('rainwise' in value for value in literal_keys)):
            item['reason']='reviewed_source_product_mismatch';continue
        groups[key].append((item,fields,name,primary))
    rules=[]
    for key,group in sorted(groups.items()):
        primary_names={product_key(name):name for _,_,name,primary in group if primary and name}
        secondary_names={product_key(name):name for _,_,name,_ in group if name}
        names=primary_names or secondary_names
        if len(names)!=1:
            for item,_,_,_ in group:item['reason']='missing_product_context' if not names else 'ambiguous_products_for_identical_signal'
            continue
        name=next(iter(names.values()))
        if not primary_names:
            related=any(product_key(name) in product_key(predicate['value'])
                        for item,_,_,_ in group
                        for predicate in query_predicates_tree(by_record[item['record_id']]['query']))
            if not related:
                for item,_,_,_ in group:item['reason']='unconfirmed_product_association'
                continue
        rule_id='wikibool-'+hashlib.sha256((product_key(name)+'\n'+key).encode()).hexdigest()[:24]
        assert rule_id not in builtin_ids
        reviewed_boolean_bad={
            'wikibool-5fc1a2dc9a9c6ee4a7e1f60b':'reviewed_generic_reporter_title_and_CSS_dimensions',
            'wikibool-5c0ba235a69cd3b8593f00d9':'reviewed_generic_router_server_and_category_not_Ricon',
            'wikibool-840a167af6812a782ae961ac':'reviewed_generic_AC_title_and_login_image_not_vendor_identity',
            'wikibool-70279e1cb750dcf48af219fc':'unconfirmed_model_family_marker',
            'wikibool-2642253e7bd01bd6285b1a74':'unconfirmed_model_family_marker',
            'wikibool-d1c7e9f1768e5c173d8fc578':'unconfirmed_model_family_marker',
            'wikibool-b17ae2881e3f5b96daf7e986':'unconfirmed_model_family_marker',
            'wikibool-213677e83a6099c7a1605c14':'ambiguous_source_escape',
        }
        if rule_id in reviewed_boolean_bad:
            for item,_,_,_ in group:item['reason']=reviewed_boolean_bad[rule_id]
            continue
        covered_by=existing_rule_coverage(name,group[0][1]['condition'],[*builtins,*existing_rules])
        if covered_by:
            for item,_,_,_ in group:item.update(status='protected_existing',reason='existing_rule_covers_all_branches',rule_id=rule_id,covered_by=covered_by)
            continue
        rules.append({'id':rule_id,'name':name,**group[0][1]})
        representative=next((item for item,_,n,primary in group if primary and n),group[0][0])
        for item,_,_,_ in group:item.update(status='converted' if item is representative else 'duplicate',reason='converted_complete_boolean_tree' if item is representative else 'same_canonical_condition_tree',rule_id=rule_id)
    return sorted(rules,key=lambda rule:rule['id']),ledger


def write_json(path,value):
    with path.open('x',encoding='utf-8') as f:json.dump(value,f,ensure_ascii=False,indent=2);f.write('\n')

def call_cli(*args):
    result=subprocess.run([sys.executable,str(CLI),*map(str,args)],capture_output=True,text=True)
    if result.returncode:raise RuntimeError(result.stderr.strip() or result.stdout.strip())
    return json.loads(result.stdout)

def generate(knowledge,history,output):
    output.mkdir(parents=True,exist_ok=False)
    protected_paths=['src-tauri/resources/rules/fingerprints.json','src-tauri/resources/rules/active_exposures.json','src-tauri/src/scanning/exposure_probe.rs','src-tauri/src/commands/scan_execution.rs','src-tauri/src/lib.rs','src-tauri/src/scanning.rs','src/features/configuration/sections/ConfigFingerprintLibrarySection.vue']
    protected_hashes={rel:sha(ROOT/rel) for rel in protected_paths}
    builtins=json.loads((ROOT/protected_paths[0]).read_text())['fingerprints']
    records,inventory,ignored,exclusions=collect_sources(knowledge,history)
    rules,ledger=convert_records(records,builtins)
    if not rules:raise RuntimeError('No current-format rules eligible; see source constraints')
    candidate=output/'candidates.json';write_json(candidate,{'schemaVersion':1,'fingerprints':rules})
    validation=output/'rust-validation.json';call_cli('validate-rules',candidate,validation)
    validation_data=json.loads(validation.read_text());bad={r['id']:r['error'] for r in validation_data['results'] if not r['valid']}
    for row in ledger:
        if row['rule_id'] in bad:
            row.update(status='rejected',reason='production_Rust_validation_failed',validation_error=bad[row['rule_id']])
    valid=[r for r in rules if r['id'] not in bad]
    canonical_input=output/'fingerprints.current.json';write_json(canonical_input,{'schemaVersion':1,'fingerprints':valid})
    pack=output/'wiki.fingerprints.json';call_cli('normalize',canonical_input,pack)
    doc=json.loads(pack.read_text());doc['manifest']={'name':'Local Wiki identification fingerprints','version':'2026-10-30','sources':[{'name':'Local historical Goby identification fields and non-image product features','revision':hashlib.sha256(compact(inventory).encode()).hexdigest()}]}
    pack.write_text(json.dumps(doc,ensure_ascii=False,indent=2)+'\n')
    pack_validation=call_cli('validate',pack);write_json(output/'pack-validation.json',pack_validation)
    native_validation=call_cli('validate-current',canonical_input);write_json(output/'native-validation.json',native_validation)
    with tempfile.TemporaryDirectory(prefix='wiki-fingerprint-native-check-') as temporary:
        directory=Path(temporary);(directory/'fingerprints.user.json').write_bytes(canonical_input.read_bytes())
        native_smoke=call_cli('smoke-current',directory,canonical_input)
    write_json(output/'native-smoke.json',native_smoke)
    write_json(output/'source-inventory.json',{'sources':inventory,'non_fingerprint_json':ignored,'non_matcher_feature_kinds_excluded':exclusions})
    with (output/'conversion-ledger.jsonl').open('x',encoding='utf-8') as f:
        for row in ledger:f.write(json.dumps(row,ensure_ascii=False)+'\n')
    def table(path,rows):
        columns=['record_id','source_kind','source_path','source_pointer','products','query_sha256','status','reason','rule_id']
        with path.open('x',encoding='utf-8',newline='') as f:
            w=csv.DictWriter(f,columns,extrasaction='ignore');w.writeheader()
            for row in rows:w.writerow({**row,'products':json.dumps(row['products'],ensure_ascii=False)})
    table(output/'unconverted.csv',[r for r in ledger if r['status']=='rejected'])
    table(output/'all-records.csv',ledger)
    status=dict(collections.Counter(r['status'] for r in ledger));reasons=dict(collections.Counter(r['reason'] for r in ledger))
    summary={'source_records':len(records),'status_counts':status,'reason_counts':reasons,'new_unique_rules':len(valid),'builtin_rules_preserved':len(builtins),'expected_effective_without_prior_user_rules':len(builtins)+len(valid),'pack_sha256':doc['sha256'],'pack_bytes':pack.stat().st_size,'non_matcher_feature_kinds_excluded':exclusions,'current_loader':'ANY-only; nontrivial AND not activated','no_target_network_requests':True,'originals_unchanged':True,'protected_code_hashes':protected_hashes,'source_inventory_sha256':sha(output/'source-inventory.json')}
    for rel,digest in protected_hashes.items():
        if sha(ROOT/rel)!=digest:raise RuntimeError('Protected source changed concurrently: '+rel)
    for source in inventory:
        if sha(Path(source['path']))!=source['sha256']:raise RuntimeError('Original source changed during conversion: '+source['path'])
    write_json(output/'summary.json',summary)
    readme=f'''# Wiki 指纹转换与当前格式导入包\n\n已检查 {len(records)} 条来源记录，生成 {len(valid)} 条去重的新识别规则。原有 {len(builtins)} 条内置规则与当前扫描代码未改写；源文件均保留。\n\n- `wiki.fingerprints.json`：独立指纹包，仅含识别字段和来源摘要，不含 CVE 库、漏洞请求、扫描结果或会话 Cookie。\n- `fingerprints.current.json`：当前代码原生格式（schemaVersion + fingerprints），仅 ANY 语义，无 matchMode 字段。\n- `summary.json`：全部记录的分类计数、校验值和受保护源码哈希。\n- `unconverted.csv`：无法转换/不应激活的记录及原因、原始定位。\n- `conversion-ledger.jsonl` / `all-records.csv`：全量去重与转换对账，原始查询只记哈希不复制敏感值。\n- `rust-validation.json` / `pack-validation.json`：调用现有 Rust 指纹包校验器的结果。\n\n当前加载器是任一信号命中，不支持 AND。AND、混合布尔、平台 app 分类引用、图标/协议/证书条件、OCR-only、低置信度、含糊产品归属、通用登录/404/公告标记及敏感 Cookie 均未强行转换。没有将 app 名称猜成正文特征，也没有删掉条件扩大命中范围。\n\n正文条件保留原字面值大小写，正则元字符全部转义；title 仅在原始 HTML title 范围匹配，不声称复现 FOFA 的实体解码、标题清洗和历史索引。所有规则都是历史来源的静态识别条件，未对真实资产验证，不是漏洞证明。\n\n原生加载器兼容性检查及真实导入结果另存 `import-result.json`（导入后生成）。可以把独立包交给支持该格式的应用；当前自助界面也可导入 `fingerprints.current.json`，但该界面的导入操作会替换用户文件，已有自定义规则时应先合并或备份。\n'''
    (output/'README.md').write_text(readme)
    return summary


def generate_boolean(knowledge,history,output,existing_path=None,prepare_only=False):
    """Build a separate additive schema-2 pack; never change the legacy file."""
    output.mkdir(parents=True,exist_ok=False)
    protected_paths=['src-tauri/resources/rules/fingerprints.json','src-tauri/resources/rules/active_exposures.json','src-tauri/src/scanning/exposure_probe.rs','src-tauri/src/scanning/fingerprint_logic.rs','src-tauri/src/scanning/fingerprint_pack.rs','src-tauri/src/commands/scan_execution.rs','src-tauri/src/lib.rs','src-tauri/src/scanning.rs','src/features/configuration/sections/ConfigFingerprintLibrarySection.vue']
    protected_hashes={rel:sha(ROOT/rel) for rel in protected_paths}
    builtins=json.loads((ROOT/protected_paths[0]).read_text())['fingerprints']
    existing=[];existing_hash=None
    if existing_path:
        existing_hash=sha(existing_path);document=json.loads(existing_path.read_text())
        existing=document['fingerprints'] if isinstance(document,dict) else document
    records,inventory,ignored,exclusions=collect_sources(knowledge,history)
    rules,ledger=convert_boolean_records(records,builtins,existing)
    if not rules:raise RuntimeError('No reliable new condition-tree rules eligible')
    candidate=output/'candidates.json'
    write_json(candidate,{'schemaVersion':2,'booleanRuntime':'condition-tree-v1','fingerprints':rules})
    valid=rules;pack=None;pack_sha256=None
    if not prepare_only:
        validation=output/'rust-validation.json';call_cli('validate-rules',candidate,validation)
        bad={row['id']:row['error'] for row in json.loads(validation.read_text())['results'] if not row['valid']}
        for row in ledger:
            if row['rule_id'] in bad:row.update(status='rejected',reason='production_Rust_validation_failed',validation_error=bad[row['rule_id']])
        valid=[rule for rule in rules if rule['id'] not in bad]
        if not valid:raise RuntimeError('No new condition rules passed production Rust validation')
    native=output/'fingerprints.boolean.current.json'
    write_json(native,{'schemaVersion':2,'booleanRuntime':'condition-tree-v1','fingerprints':valid})
    if not prepare_only:
        pack=output/'wiki.boolean.fingerprints.json';call_cli('normalize-boolean',native,pack)
        document=json.loads(pack.read_text());document['manifest']={'name':'Local Wiki boolean identification fingerprints','version':'2026-10-30','sources':[{'name':'Local historical Goby identification fields and non-image product features','revision':hashlib.sha256(compact(inventory).encode()).hexdigest()}]}
        pack.write_text(json.dumps(document,ensure_ascii=False,indent=2)+'\n');pack_sha256=document['sha256']
        write_json(output/'pack-validation.json',call_cli('validate',pack))
        write_json(output/'native-validation.json',call_cli('validate-current',native))
        with tempfile.TemporaryDirectory(prefix='wiki-boolean-native-check-') as temporary:
            directory=Path(temporary);(directory/'fingerprints.boolean.json').write_bytes(native.read_bytes())
            write_json(output/'native-smoke.json',call_cli('smoke-current',directory,native))
    write_json(output/'source-inventory.json',{'sources':inventory,'non_fingerprint_json':ignored,'non_matcher_feature_kinds_excluded':exclusions})
    with (output/'conversion-ledger.jsonl').open('x',encoding='utf-8') as f:
        for row in ledger:f.write(json.dumps(row,ensure_ascii=False)+'\n')
    columns=['record_id','source_kind','source_path','source_pointer','products','query_sha256','status','reason','rule_id','supportive_conditions','covered_by']
    def table(path,rows):
        with path.open('x',encoding='utf-8',newline='') as f:
            writer=csv.DictWriter(f,columns,extrasaction='ignore');writer.writeheader()
            for row in rows:writer.writerow({**row,'products':json.dumps(row['products'],ensure_ascii=False),'supportive_conditions':json.dumps(row.get('supportive_conditions',[]),ensure_ascii=False),'covered_by':json.dumps(row.get('covered_by',[]),ensure_ascii=False)})
    table(output/'all-records.csv',ledger)
    table(output/'unconverted.csv',[row for row in ledger if row['status']=='rejected'])
    summary={'source_records':len(records),'status_counts':dict(collections.Counter(row['status'] for row in ledger)),'reason_counts':dict(collections.Counter(row['reason'] for row in ledger)),'new_unique_rules':len(valid),'builtin_rules_preserved':len(builtins),'existing_rules_readonly':len(existing),'existing_rules_sha256':existing_hash,'new_rules_only':True,'booleanRuntime':'condition-tree-v1','pack_sha256':pack_sha256,'pack_bytes':pack.stat().st_size if pack else None,'native_bytes':native.stat().st_size,'prepare_only':prepare_only,'no_target_network_requests':True,'originals_unchanged':True,'protected_code_hashes':protected_hashes,'source_inventory_sha256':sha(output/'source-inventory.json')}
    for rel,digest in protected_hashes.items():
        if sha(ROOT/rel)!=digest:raise RuntimeError('Protected source changed concurrently: '+rel)
    for source in inventory:
        if sha(Path(source['path']))!=source['sha256']:raise RuntimeError('Original source changed during conversion: '+source['path'])
    if existing_path and sha(existing_path)!=existing_hash:raise RuntimeError('Existing fingerprint file changed during conversion')
    write_json(output/'summary.json',summary)
    (output/'README.md').write_text(f"""# 增量布尔指纹包\n\n已检查 {len(records)} 条来源记录，生成 {len(valid)} 条新增条件树指纹。原有 Any 文件和内置规则未重写；本目录不执行真实导入，不访问 Oviraptor 数据库。\n\n条件以 all/any 树完整保留，不通过删除条件凑数量。敏感会话、CVE/利用字面值、平台 app 标签、图标/协议、未知字段与否定条件仍不自动导入。通用登录页、404、缓存头、共用脚本只能作为品牌条件的必要辅助条件，不能形成独立 OR 分支。\n\n- `fingerprints.boolean.current.json`：schemaVersion 2、booleanRuntime=condition-tree-v1，专用于新运行时的独立 `fingerprints.boolean.json`。不要导入旧版 Any 加载器或覆盖 `fingerprints.user.json`。\n- `wiki.boolean.fingerprints.json`：经过生产规范化器生成的分享包（准备模式不生成）。\n- `summary.json` / `all-records.csv` / `unconverted.csv`：逐条对账、去重、未转换原因。\n- `native-validation.json` / `native-smoke.json`：当前运行代码的离线校验结果（准备模式不生成）。\n\n所有条件来自静态历史材料，尚未对真实资产逐条验证。标题条件限定原始 HTML title 范围，不声称复现 FOFA 的实体解码和历史索引。\n""")
    return summary


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--knowledge-package',required=True,type=Path);p.add_argument('--history-root',required=True,type=Path);p.add_argument('--output',required=True,type=Path)
    p.add_argument('--boolean-only',action='store_true',help='Only incremental complete condition trees; never rewrite old Any fingerprints')
    p.add_argument('--existing-rules',type=Path,help='Read-only existing catalog used to avoid duplicate condition signatures')
    p.add_argument('--prepare-only',action='store_true',help='Build candidates and ledger without invoking Rust validators; not for live import')
    a=p.parse_args()
    if a.prepare_only and not a.boolean_only:p.error('--prepare-only requires --boolean-only')
    result=(generate_boolean(a.knowledge_package.resolve(),a.history_root.resolve(),a.output.resolve(),a.existing_rules.resolve() if a.existing_rules else None,a.prepare_only) if a.boolean_only else generate(a.knowledge_package.resolve(),a.history_root.resolve(),a.output.resolve()))
    print(json.dumps(result,ensure_ascii=False,indent=2))
if __name__=='__main__':main()
