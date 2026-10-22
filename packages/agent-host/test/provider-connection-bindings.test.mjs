import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { AgentHost } from '../src/service.mjs';
import { ProviderStore, OpenAiCompatibleRuntime } from '../src/provider.mjs';
import { serveHost } from '../src/server.mjs';
const ref = (id='a') => ({policyId:`policy-${id}`,revision:1,kind:'models',modelProfileId:`model-${id}`});
const local = {provider:'openai-compatible',baseUrl:'http://127.0.0.1:8123/v1',model:'fixture'};
async function fixture(t) { const dir=await mkdtemp(join(await realpath(tmpdir()),'ovi-model-binding-'));t.after(()=>rm(dir,{recursive:true,force:true}));return new ProviderStore(dir); }

test('a different catalog connection at the same URL cannot borrow the active key',async t=>{
 const store=await fixture(t);
 store.save({...local,apiKey:'synthetic-key-one',authType:'api-key',connectionRef:ref()});
 assert.throws(()=>store.save({...local,authType:'api-key',connectionRef:ref('b')}),{code:'host_provider_api_key_required'});
 assert.deepEqual(store.publicView().connectionRef,ref());assert.equal(store.bearer(),'synthetic-key-one');
 store.save({...local,apiKey:'synthetic-key-two',authType:'api-key',connectionRef:ref('b')});
 assert.deepEqual(store.publicView().connectionRef,ref('b'));assert.equal(store.bearer(),'synthetic-key-two');
 store.save({...local,model:'other'});assert.equal(store.publicView().connectionRef,null,'editing outside the catalog invalidates its identity');
 assert.throws(()=>store.save({...local,apiKey:'synthetic-key',connectionRef:{...ref(),revision:0}}),{code:'host_provider_connection_ref_invalid'});
 assert(!JSON.stringify(store.publicView()).includes('synthetic-key'));
});

test('explicit keyless selection clears the selected key and does not reuse environment auth',async t=>{
 const store=await fixture(t);
 store.save({...local,apiKey:'synthetic-old-key',connectionRef:ref()});
 assert.throws(()=>store.save({...local,authType:'none',connectionRef:ref('b')}),{code:'host_provider_auth_type_invalid'});
 store.save({...local,authType:'none',clearApiKey:true,connectionRef:ref('b')});
 assert.equal(store.bearer(),null);assert.equal(store.read().apiKey,undefined);
 assert.equal(store.publicView().configured,true);assert.equal(store.publicView().authType,'none');
 store.save({...local,model:'edited'});assert.equal(store.bearer(),null);assert.equal(store.publicView().authType,'none');
 assert.throws(()=>store.save({...local,authType:'api-key',connectionRef:ref('c')}),{code:'host_provider_api_key_required'});
});

test('real Host HTTP settings activate explicit no-auth and runtime sends no Bearer in chat or responses',async t=>{
 const store=await fixture(t),requests=[];
 const host=new AgentHost({dataDir:store.dir.replace(/\/secrets$/,''),networkEnvironment:{},providerFetch:async(url,init)=>{
  requests.push({url,headers:init.headers});return {ok:true,status:200,body:(async function*(){yield 'data: {"choices":[{"delta":{"content":"Fixture response"},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n';})()};
 }});
 const web=await serveHost(host);t.after(async()=>{await web.close();await host.close();});
 const post=(path,body)=>fetch(web.origin+path,{method:'POST',headers:{'X-Oviraptor-Session':web.token,'Content-Type':'application/json'},body:JSON.stringify(body)});
 const response=await post('/api/settings/provider',{...local,authType:'none',clearApiKey:true,connectionRef:ref()});
 assert.equal(response.status,200);const view=(await response.json()).apiProvider;
 assert.equal(view.authType,'none');assert.deepEqual(view.connectionRef,ref());
 assert.equal((await post('/api/settings',{runtime:'xai-api'})).status,200);
 assert.equal(host.runtime.config.authentication,'none');
 const session=await host.runtime.newSession();await host.runtime.prompt({sessionId:session.sessionId,prompt:'Say hello'});
 assert.equal(requests.length,1);assert.equal(Object.keys(requests[0].headers).some(name=>name.toLowerCase()==='authorization'),false);
 let headers,responseUrl;
 const runtime=new OpenAiCompatibleRuntime({config:{provider:"xai",baseUrl:"https://api.x.ai/v1",model:'grok-4.7',authentication:'none'},fetch:async(url,init)=>{responseUrl=url;headers=init.headers;return{ok:false,status:401};}});
 const active=await runtime.newSession();await assert.rejects(runtime.prompt({sessionId:active.sessionId,prompt:'hello'}));
 assert.match(responseUrl,/\/responses$/);assert.equal(Object.keys(headers).some(name=>name.toLowerCase()==='authorization'),false);
});

const deviceResponse=()=>new Response(JSON.stringify({device_code:'synthetic-device',user_code:'TEST-CODE',verification_uri:'https://example.test/device',expires_in:300}),{status:200});
const deferred=()=>{let resolve;const promise=new Promise(done=>{resolve=done;});return{promise,resolve};};
test('late device-code start cannot overwrite an explicitly selected connection',async t=>{
 const store=await fixture(t),entered=deferred(),release=deferred();
 const host=new AgentHost({dataDir:store.dir.replace(/\/secrets$/,''),networkEnvironment:{},providerFetch:async()=>{entered.resolve();return release.promise;}});t.after(()=>host.close());
 const login=host.startProviderOAuth({background:false});await entered.promise;
 await host.setProvider({...local,apiKey:'synthetic-new-key',authType:'api-key',connectionRef:ref()});
 release.resolve(deviceResponse());await assert.rejects(login,{code:'host_oauth_cancelled'});
 assert.equal(host.oauthPending,null);assert.equal(host.providerStore.bearer(),'synthetic-new-key');assert.deepEqual(host.settings().apiProvider.connectionRef,ref());
});
test('late manual OAuth success after a connection switch cannot replace API credentials or runtime',async t=>{
 const store=await fixture(t),entered=deferred(),release=deferred();let count=0;
 const host=new AgentHost({dataDir:store.dir.replace(/\/secrets$/,''),networkEnvironment:{},providerFetch:async()=>{if(++count===1)return deviceResponse();entered.resolve();return release.promise;}});t.after(()=>host.close());
 await host.startProviderOAuth({background:false});const polling=host.pollProviderOAuth();await entered.promise;
 await host.setProvider({...local,apiKey:'synthetic-chosen-key',connectionRef:ref('b')});await host.setRuntime('xai-api');
 release.resolve(new Response(JSON.stringify({access_token:'synthetic-late-oauth',refresh_token:'synthetic-refresh'}),{status:200}));await assert.rejects(polling,{code:'host_oauth_cancelled'});
 assert.equal(host.providerStore.bearer(),'synthetic-chosen-key');assert.equal(host.runtime.config.apiKey,'synthetic-chosen-key');assert.equal(host.settings().apiProvider.oauthSaved,false);
});
