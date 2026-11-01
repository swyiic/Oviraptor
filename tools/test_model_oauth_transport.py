#!/usr/bin/env python3
"""Production resolver/transport regressions using synthetic loopback only.

Extracts pure source boundaries so no Tauri launch, real database, OAuth login,
keychain or upstream request is needed. Cached Cargo dependencies are required.
"""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
NAMES = ('serde', 'serde_json', 'rusqlite', 'reqwest', 'tokio', 'chrono')
PRELUDE = r'''
#![allow(dead_code,unused_imports)]
mod secret_store {
 pub struct SecretStore;
 impl SecretStore {pub fn system()->Self{Self} pub fn resolve(&self,_id:&str,value:Option<&str>)->Option<String>{value.map(str::to_string)}}
 pub fn llm_local_api_key(scope:&str)->String{scope.into()} pub fn llm_api_key(scope:&str)->String{scope.into()}
}
mod configuration_catalog {
 pub fn read_current_defaults(_: &rusqlite::Connection)->Result<serde_json::Value,String>{Err("fixture must not read DB".into())}
 pub fn read_current_defaults_for(_: &rusqlite::Connection,_:&[&str])->Result<serde_json::Value,String>{Err("fixture must not read DB".into())}
}
'''
TESTS = r'''
use model_configuration::{model_connection,gateway_profile,NormalizedModelSettings};
use serde_json::json;
fn oauth()->serde_json::Value{json!({"id":"grok","llm":"grok-4.7","authType":"oauth","deployment":"cloud","apiBase":"https://api.x.ai/v1","apiKey":"","localApiKey":""})}
fn resolve(p:serde_json::Value)->Result<model_configuration::ModelConnection,String>{model_connection(&NormalizedModelSettings::new(&json!({"modelProfiles":[p],"activeModelProfileId":"grok","_configurationRefs":{"models":{}}})))}
fn register(port:u16){agent_host_sidecar::set(Some(agent_host_sidecar::AgentHostEndpoint{origin:format!("http://127.0.0.1:{port}"),session:"fixture-session-not-oauth".into(),source:"fixture".into()}));}
#[test] fn oauth_snapshot_is_credential_free_and_host_unavailable_fails_closed(){
 agent_host_sidecar::set(None);let env=resolve(oauth()).unwrap();assert_eq!(env.api_base,"oviraptor://grok-oauth");assert_eq!(env.api_key,"");
 assert!(!serde_json::to_string(&env).unwrap().contains("fixture-session"));assert!(gateway_profile(&env,None).unwrap_err().contains("Agent Host"));
}
#[test] fn oauth_rejects_foreign_endpoints_keys_local_and_invalid_auth(){
 for (key,value) in [("apiBase",json!("https://foreign.example/v1")),("apiKey",json!("unexpected")),("localApiKey",json!("unexpected")),("deployment",json!("local")),("authType",json!("unknown")),("authType",json!(12))]{let mut p=oauth();p[key]=value;assert!(resolve(p).is_err());}
}
#[test] fn api_and_local_resolution_keep_existing_behavior(){
 let p=json!({"id":"grok","llm":"fixture","deployment":"cloud","apiBase":"https://model.example/v1","apiKey":"fixture-api"});let env=resolve(p).unwrap();assert_eq!(env.api_key,"fixture-api");assert_eq!(gateway_profile(&env,Some("http://127.0.0.1:9009")).unwrap().endpoint,"https://model.example/v1/chat/completions");
 let p=json!({"id":"grok","llm":"fixture","deployment":"local","apiBase":"http://127.0.0.1:9008/v1","apiKey":"stale-cloud-key","localApiKey":"local-only"});let env=resolve(p).unwrap();assert_eq!(env.api_key,"local-only");assert!(gateway_profile(&env,None).unwrap().local);
}
#[test] fn logical_profile_uses_current_host_session_only_at_gateway_boundary(){
 let env=resolve(oauth()).unwrap();register(9009);let profile=gateway_profile(&env,Some("http://proxy.example:9876")).unwrap();
 assert_eq!(profile.endpoint,"http://127.0.0.1:9009/api/model/oauth/chat/completions");assert_eq!(profile.bearer(),Some("fixture-session-not-oauth"));assert!(profile.proxy.is_none());assert!(!profile.local);
 assert_eq!(env.api_base,"oviraptor://grok-oauth");assert_eq!(env.api_key,"");agent_host_sidecar::set(None);assert!(gateway_profile(&env,None).is_err());
}
fn capture(reply:&'static str)->(u16,std::sync::mpsc::Receiver<String>,std::thread::JoinHandle<()>) {
 use std::io::{Read,Write};let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();let(tx,rx)=std::sync::mpsc::channel();
 let join=std::thread::spawn(move||{let(mut socket,_)=listener.accept().unwrap();socket.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();let mut raw=Vec::new();let mut buf=[0;4096];loop{let n=socket.read(&mut buf).unwrap();if n==0{break}raw.extend_from_slice(&buf[..n]);if raw.windows(4).any(|w|w==b"\r\n\r\n"){break}}
 tx.send(String::from_utf8_lossy(&raw).into_owned()).unwrap();socket.write_all(reply.as_bytes()).unwrap();});(port,rx,join)
}
#[test] fn oauth_gateway_reaches_loopback_with_session_and_bypasses_configured_proxy(){
 let(port,rx,join)=capture("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");register(port);
 let env=resolve(oauth()).unwrap();let profile=gateway_profile(&env,Some("http://127.0.0.1:1")).unwrap();
 let mut request=transport::TransportRequest::post(profile.endpoint.clone(),b"{}".to_vec()).header("Authorization",format!("Bearer {}",profile.bearer().unwrap()));request.proxy=Some("http://127.0.0.1:1".into());request.total_timeout=Some(std::time::Duration::from_secs(3));
 let reply=transport::send(request,std::sync::Arc::new(||false),Box::new(|_|true)).unwrap();assert_eq!(reply.status,200);let head=rx.recv().unwrap();assert!(head.starts_with("POST /api/model/oauth/chat/completions "));assert!(head.to_lowercase().contains("authorization: bearer fixture-session-not-oauth"));join.join().unwrap();agent_host_sidecar::set(None);
}
#[test] fn oauth_bridge_rejects_redirect_even_when_caller_default_allows_it(){
 let(port,_rx,join)=capture("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");register(port);let profile=gateway_profile(&resolve(oauth()).unwrap(),None).unwrap();
 let mut request=transport::TransportRequest::post(profile.endpoint.clone(),Vec::new()).header("Authorization",format!("Bearer {}",profile.bearer().unwrap()));request.total_timeout=Some(std::time::Duration::from_secs(3));
 let reply=transport::send(request,std::sync::Arc::new(||false),Box::new(|_|true)).unwrap();assert_eq!(reply.status,302);join.join().unwrap();agent_host_sidecar::set(None);
}
'''

def main():
    for kind in ('debug', 'release'):
        deps = ROOT / f'src-tauri/target/{kind}/deps'
        libs = {name: sorted(deps.glob(f'lib{name}-*.rlib'), key=lambda p:p.stat().st_mtime_ns, reverse=True) for name in NAMES}
        if all(libs.values()): break
    else: raise RuntimeError('Cached Rust dependencies required')
    sidecar = (ROOT/'src-tauri/src/agent_host_sidecar.rs').read_text()
    registry = sidecar[sidecar.index('static MODEL_HOST_ENDPOINT:'):sidecar.index('pub struct AgentHostSidecar')]
    resolver = (ROOT/'src-tauri/src/model_configuration.rs').read_text().split('#[cfg(test)]')[0]
    resolver = '\n'.join(line for line in resolver.splitlines() if not line.startswith('//!'))
    base = ROOT/'src-tauri/src'
    with tempfile.TemporaryDirectory(prefix='ovi-oauth-transport-') as temp:
        temp=Path(temp); adapter=temp/'transport.rs';binary=temp/'transport-tests'
        adapter.write_text(PRELUDE+f'#[path="{base}/db_settings.rs"] mod db;\n'+'''
mod agent_host_sidecar {use std::sync::Mutex;
#[derive(Clone)]pub struct AgentHostEndpoint{pub origin:String,pub session:String,pub source:String}
'''+registry+'''pub fn set(value:Option<AgentHostEndpoint>){*MODEL_HOST_ENDPOINT.lock().unwrap()=value;}}\n'''+f'''
mod agent_runtime {{pub mod model {{
pub mod usage {{pub const CHARS_PER_TOKEN:usize=4;}}
#[path="{base}/agent_runtime/model/profile.rs"]mod profile;
pub use profile::{{GatewayProfile,LocalResourcePolicy,ResolvedModelProfile}};
}}}}
#[path="{base}/agent_runtime/model/transport.rs"]mod transport;
mod model_configuration {{
'''+resolver+'\n}\n'+TESTS)
        command=['rustc','--edition=2021','--test',str(adapter),'-L',f'dependency={deps}','-o',str(binary)]
        for name in NAMES: command += ['--extern',f'{name}={libs[name][0]}']
        subprocess.run(command,check=True)
        subprocess.run([str(binary),'--test-threads=1'],check=True)

if __name__=='__main__': main()
