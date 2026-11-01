#!/usr/bin/env python3
"""Compile current catalog/normalization code; test only isolated in-memory SQLite.

No app startup, real database, keychain, OAuth sign-in or network is involved.
Unrelated legacy profile readers are stubbed; production independent_append/read,
descriptor validation and normalization execute unchanged.
"""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
NAMES = ('rusqlite', 'serde_json', 'serde', 'sha2', 'uuid', 'reqwest')
PRELUDE = r'''
#![allow(dead_code)]
mod configuration_policies {
 #[derive(Clone,serde::Serialize,serde::Deserialize,PartialEq,Debug)]
 pub struct PolicyReference { pub policy_id:String, pub revision:i64, pub kind:String }
 pub fn read_profile(_: &rusqlite::Connection,_:i64)->Result<serde_json::Value,String>{Err("unused legacy reader".into())}
 pub fn reference(_: &serde_json::Value,_:&str)->Result<Option<PolicyReference>,String>{Err("unused legacy reference".into())}
}
mod configuration_json {pub fn equivalent(a:&serde_json::Value,b:&serde_json::Value)->bool{a==b}}
'''
TESTS = r'''
#[cfg(test)] mod catalog_tests {
 use super::*;
 fn db()->Connection {let db=Connection::open_in_memory().unwrap();db.execute_batch("BEGIN IMMEDIATE").unwrap();db}
 fn oauth()->Value {json!({"id":"grok","name":"Grok OAuth","llm":"grok-4.7","deployment":"cloud","authType":"oauth","apiBase":"https://api.x.ai/v1","apiKey":"","localApiKey":""})}
 fn api()->Value {json!({"id":"api","name":"API","llm":"deepseek-chat","deployment":"cloud","apiBase":"https://model.example.invalid/v1","apiKey":"fixture-key","localApiKey":""})}
 fn local()->Value {json!({"id":"local","name":"Local","llm":"local-model","deployment":"local","apiBase":"http://127.0.0.1:11434/v1","apiKey":"","localApiKey":""})}
 fn settings(models:Vec<Value>,active:&str)->Value{json!({"modelProfiles":models,"activeModelProfileId":active})}
 #[test] fn oauth_roundtrip_has_no_credential_row(){
  let db=db();let r=independent_append(&db,"models",&settings(vec![oauth()],"grok"),None).unwrap();
  let result=independent_read(&db,&r).unwrap();assert_eq!(result["modelProfiles"][0],oauth());assert_eq!(result["modelAuthType"],"oauth");
  for table in ["configuration_service_accounts","configuration_model_connections"]{let n:i64=db.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get(0)).unwrap();assert_eq!(n,0);}
  let text:String=db.query_row("SELECT descriptor_json FROM configuration_connection_versions",[],|r|r.get(0)).unwrap();assert!(!text.contains("access_token"));assert!(!text.contains("oviraptor://"));
 }
 #[test] fn mixed_catalog_preserves_all_models_and_active_selection(){
  let db=db();let a=independent_append(&db,"models",&settings(vec![api()],"api"),None).unwrap();
  let b=independent_append(&db,"models",&settings(vec![api(),oauth(),local()],"api"),Some(&a)).unwrap();
  assert_eq!(independent_read(&db,&a).unwrap()["modelProfiles"].as_array().unwrap().len(),1);
  let result=independent_read(&db,&b).unwrap();assert_eq!(result["activeModelProfileId"],"api");assert_eq!(result["modelProfiles"].as_array().unwrap().len(),3);assert_eq!(result["modelApiKey"],"fixture-key");
  let c=independent_append(&db,"models",&settings(vec![api(),oauth(),local()],"grok"),Some(&b)).unwrap();
  let result=independent_read(&db,&c).unwrap();assert_eq!(result["modelAuthType"],"oauth");assert_eq!(result["modelApiKey"],"");assert_eq!(result["modelApiBase"],"https://api.x.ai/v1");
 }
 #[test] fn local_model_preserves_no_auth_cache(){let db=db();let r=independent_append(&db,"models",&settings(vec![local()],"local"),None).unwrap();let result=independent_read(&db,&r).unwrap();assert_eq!(result["modelAuthType"],"none");assert_eq!(result["localApiKey"],"");}
 #[test] fn oauth_rejects_foreign_endpoints_credentials_and_deployment(){
  let mut cases=vec![];
  for (field,value) in [("apiBase","https://elsewhere.example/v1"),("apiKey","token-must-not-be-stored"),("localApiKey","unexpected"),("deployment","local"),("llm","")]{let mut p=oauth();p[field]=json!(value);cases.push(p);}
  for p in cases {let db=db();assert!(independent_append(&db,"models",&settings(vec![p],"grok"),None).is_err());}
 }
 #[test] fn unknown_or_malformed_auth_is_rejected(){for value in [json!("mystery"),json!(123),Value::Null]{let db=db();let mut p=api();p["authType"]=value;assert!(independent_append(&db,"models",&settings(vec![p],"api"),None).is_err());}}
 #[test] fn api_without_key_never_borrows_saved_oauth(){let db=db();let mut p=api();p["apiKey"]=json!("");assert!(independent_append(&db,"models",&settings(vec![oauth(),p],"api"),None).is_err());}
 #[test] fn stale_revision_cannot_replace_new_catalog(){let db=db();let a=independent_append(&db,"models",&settings(vec![api()],"api"),None).unwrap();let _b=independent_append(&db,"models",&settings(vec![api(),oauth()],"grok"),Some(&a)).unwrap();assert_eq!(independent_append(&db,"models",&settings(vec![api()],"api"),Some(&a)).unwrap_err(),"configuration_category_head_conflict");}
 #[test] fn oauth_descriptor_cannot_smuggle_api_reference(){let db=db();db.execute_batch(SCHEMA).unwrap();let mut d=write_descriptor(&db,"models",&settings(vec![oauth()],"grok")).unwrap();d["entries"][0]["connectionId"]=json!("other-key");assert!(materialize(&db,"models",&d).is_err());}
 #[test] fn oauth_descriptor_identity_cannot_drift(){let db=db();db.execute_batch(SCHEMA).unwrap();let mut d=write_descriptor(&db,"models",&settings(vec![oauth()],"grok")).unwrap();d["entries"][0]["oauth"]["id"]=json!("different");assert!(materialize(&db,"models",&d).is_err());}
 #[test] fn duplicate_ids_and_missing_active_model_rejected(){let db=db();assert!(independent_append(&db,"models",&settings(vec![oauth(),oauth()],"grok"),None).is_err());assert!(independent_append(&db,"models",&settings(vec![oauth()],"missing"),None).is_err());}
}
'''
NORMALIZE_TESTS = r'''
#[test] fn flat_oauth_normalization_retains_authentication(){let value=normalization::normalize_settings(&serde_json::json!({"modelName":"grok-4.7","modelApiBase":"https://api.x.ai/v1","modelAuthType":"oauth","modelApiKey":""}));assert_eq!(value["modelProfiles"][0]["authType"],"oauth");assert_eq!(value["modelProfiles"][0]["apiKey"],"");}
#[test] fn explicit_model_list_wins_over_flat_oauth_cache(){let raw=serde_json::json!({"modelProfiles":[],"modelName":"grok-4.7","modelApiBase":"https://api.x.ai/v1","modelAuthType":"oauth"});assert_eq!(normalization::normalize_settings(&raw),raw);}
'''

def main():
    for kind in ('debug', 'release'):
        deps = ROOT / f'src-tauri/target/{kind}/deps'
        libraries = {n: sorted(deps.glob(f'lib{n}-*.rlib'), key=lambda p:p.stat().st_mtime_ns, reverse=True) for n in NAMES}
        if all(libraries.values()): break
    else: raise RuntimeError('Cached Rust dependencies required')
    source = (ROOT/'src-tauri/src/configuration/connections.rs').read_text()
    source = '\n'.join(line for line in source.splitlines() if not line.startswith('//!'))
    normalization = ROOT/'src-tauri/src/db_settings.rs'
    with tempfile.TemporaryDirectory(prefix='ovi-oauth-catalog-') as temp:
        temp=Path(temp); adapter=temp/'catalog.rs';binary=temp/'catalog-tests'
        adapter.write_text(PRELUDE+'\nmod connections {\n'+source+'\n'+TESTS+'\n}\n'+f'#[path="{normalization}"] mod normalization;\n'+NORMALIZE_TESTS)
        command=['rustc','--edition=2021','--test',str(adapter),'-L',f'dependency={deps}','-o',str(binary)]
        for name in NAMES: command += ['--extern',f'{name}={libraries[name][0]}']
        subprocess.run(command,check=True)
        subprocess.run([str(binary)],check=True)

if __name__=='__main__':main()
