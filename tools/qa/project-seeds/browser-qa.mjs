// Local rendered-layout regression. Uses the real seed SFC, shell chrome and
// navigation. The read-only native transport is scripted; no database, model,
// target or external-provider acceptance is implied.
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import {createServer} from 'vite';
import vue from '@vitejs/plugin-vue';
const require=createRequire(import.meta.url);
const {chromium}=require(process.env.OVI_PLAYWRIGHT_MODULE||'playwright');
const repo=fileURLToPath(new URL('../../../',import.meta.url));
const entry='\0seed-layout-entry',transport='\0seed-layout-transport';
const fixture={name:'seed-layout-fixture',enforce:'pre',resolveId(source){
 if(source===entry)return entry;
 if(source.startsWith('@tauri-apps/'))return transport;
},load(id){
 if(id===transport)return 'export const invoke=(...args)=>window.__seedLayoutInvoke(...args);export const listen=()=>{throw Error("unexpected native subscription")};export const openUrl=()=>{throw Error("unexpected external link")};';
 if(id!==entry)return;
 return `import {createApp,h} from 'vue';
 import Seeds from '/src/features/assets/components/ProjectSeedsWorkspace.vue';
 import Navigation from '/src/shell/WorkspaceSectionNavigation.vue';
 import '/src/shell/styles/chrome.css';import '/src/shell/styles/desktop-theme.css';import '/src/shell/agent-shell.css';import '/src/shell/styles/focused-navigation.css';
 const source=Array.from({length:50},(_,i)=>({id:i+1,kind:i?'domain':'fingerprint',value:i?'host'+i+'.example.com':'0123456789abcdef'.repeat(4),trustStatus:'needs_evidence',trustReason:'No complete host-bound ownership evidence; AI advice alone is not verified ownership.',firstSeenRound:null,firstSeenAt:null,lastSeenAt:'2026-10-23T01:02:03Z',historicalBaseline:true,sourceTypes:['fofa','historical_baseline'],assetCount:3,observationCount:20,hunterEligible:false}));
 window.__seedLayoutCalls=[];
 const nativeObserver=window.ResizeObserver;window.__seedLayoutObservers=[];
 window.ResizeObserver=class extends nativeObserver{constructor(callback){super(callback);this.audit={observed:[],disconnected:false};window.__seedLayoutObservers.push(this.audit);}observe(element,...options){this.audit.observed.push(element.className);return super.observe(element,...options);}disconnect(){this.audit.disconnected=true;return super.disconnect();}};
 const addListener=window.addEventListener.bind(window),removeListener=window.removeEventListener.bind(window),layoutListeners=new Set();
 window.addEventListener=(name,callback,...options)=>{if(callback?.name==='updateLayout')layoutListeners.add(callback);return addListener(name,callback,...options);};
 window.removeEventListener=(name,callback,...options)=>{if(callback?.name==='updateLayout')layoutListeners.delete(callback);return removeListener(name,callback,...options);};
 window.__seedLayoutListenerCount=()=>layoutListeners.size;
 window.__seedLayoutInvoke=async(command,args)=>{window.__seedLayoutCalls.push([command,args]);
  if(command==='list_project_seeds')return {projectId:7,ready:true,page:1,pageSize:50,total:50,items:source,summary:{total:50,needsEvidence:50,verified:0,conflicts:0,excluded:0,historicalBaseline:50},rounds:[]};
  if(command==='read_project_seed')return {projectId:7,seed:source.find(row=>row.id===args.seedId),observationsTotal:40,page:args.page,pageSize:20,observations:Array.from({length:20},(_,i)=>({id:args.page*20+i,sourceType:'history',round:null,seenAt:'2026-10-23T01:02:03Z',sourceRef:'read-only-example-'+i,evidence:{original:'exact observed evidence '.repeat(12)}}))};
  throw Error('Unexpected native command '+command);
 };
 const app=createApp({render:()=>h('div',{class:'app-shell agent-shell sidebar-rail'},[h('main',{class:'main-area'},[
 h('header',{class:'topbar'},'Read-only project seed layout fixture'),h(Navigation,{module:'asset',view:'seeds',sentinelMenu:'',runningJobs:0,blockedCount:37323}),h('div',{class:'content-area'},[h(Seeds,{projectId:7})])])])});app.mount('#app');window.__seedLayoutUnmount=()=>app.unmount();`;
},configureServer(server){server.middlewares.use((request,response,next)=>{
 if(request.url!=='/')return next();response.setHeader('Content-Type','text/html');response.end('<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><style>:root{--app-bg:#fbfbfa;--app-surface:#fff;--app-ink:#24272c;--app-muted:#697586;--blue:#2878ff;--line:#e6e9ef;--text:#24272c}*{box-sizing:border-box}body{margin:0}.button{cursor:pointer;border:1px solid var(--line);background:white;padding:7px 10px;min-height:32px;color:inherit}.button.primary{background:var(--blue);color:#fff}</style></head><body><div id="app"></div><script type="module" src="/@id/__x00__seed-layout-entry"></script></body></html>');
});}};
const server=await createServer({configFile:false,root:repo,publicDir:false,plugins:[fixture,vue()],server:{host:'127.0.0.1',port:0,strictPort:false,fs:{allow:[repo]},watch:{ignored:['**/src-tauri/**']}}});
let browser;
const results=[];
try{
 await server.listen();const origin=`http://127.0.0.1:${server.httpServer.address().port}`;
 browser=await chromium.launch({headless:true,...(process.env.OVI_BROWSER_EXECUTABLE?{executablePath:process.env.OVI_BROWSER_EXECUTABLE}:{})});
 const context=await browser.newContext({viewport:{width:1700,height:1000}}),blocked=[],errors=[];
 await context.route('**/*',route=>new URL(route.request().url()).origin===origin?route.continue():(blocked.push(route.request().url()),route.abort()));
 const page=await context.newPage();page.setDefaultTimeout(8000);page.on('pageerror',error=>{errors.push(String(error));console.error('BROWSER',String(error));});page.on('console',message=>{if(message.type()==='error')console.error('CONSOLE',message.text());});await page.goto(origin);
 await page.locator('[data-seed-detail="1"]').click();await page.locator('.seed-history article').first().waitFor();
 const rect=selector=>page.locator(selector).evaluate(element=>{const box=element.getBoundingClientRect();return {top:box.top,bottom:box.bottom,left:box.left,right:box.right,height:box.height,scrollHeight:element.scrollHeight,clientHeight:element.clientHeight,scrollTop:element.scrollTop,position:getComputedStyle(element).position};});
 const assertDetail=async label=>{
  const detail=await rect('.seed-detail'),nav=await rect('.workspace-section-navigation');
  assert(detail.top>=nav.bottom+10,`${label}: detail header overlaps sticky navigation ${JSON.stringify({detail,nav})}`);assert(detail.bottom<=page.viewportSize().height-10,`${label}: details overflow viewport`);
  assert(detail.left>=0&&detail.right<=page.viewportSize().width,`${label}: details overflow viewport width`);
  assert.equal(await page.locator('.detail-seed-value').textContent(),'0123456789abcdef'.repeat(4),`${label}: selected value changed`);
  results.push({label,detail,nav});
 };
 await assertDetail('wide initial selection');
 await page.evaluate(()=>window.scrollTo(0,document.body.scrollHeight));await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));await assertDetail('wide outer scroll');
 const detailBefore=await rect('.seed-detail'),history=await rect('.seed-detail-scroll');assert(history.scrollHeight>history.clientHeight,'Long history must scroll inside details');
 await page.evaluate(()=>window.scrollBy(0,-350));await page.evaluate(()=>window.scrollTo(0,document.body.scrollHeight));
 const detailAfter=await rect('.seed-detail');assert(Math.abs(detailBefore.top-detailAfter.top)<1,'Scrolling seed rows moved details out of view');
 
 await page.locator('.seed-detail-scroll').evaluate(element=>{element.scrollTop=element.scrollHeight;});
 assert((await rect('.seed-detail-scroll')).scrollTop>0,'Detail history is independently scrollable');
 assert((await rect('.seed-detail > header')).top>=(await rect('.seed-detail')).top,'Detail controls scroll out with history');
 await assertDetail('wide independent scroll');
 const oldNavigation=await rect('.workspace-section-navigation');
 await page.locator('.main-area').evaluate(element=>{element.style.transition='none';element.style.marginLeft='1020px';});
 await page.waitForFunction(()=>document.querySelector('.workspace-section-navigation').getBoundingClientRect().height>52);
 await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
 assert((await rect('.workspace-section-navigation')).height>oldNavigation.height,'Sidebar width change did not wrap shell navigation as intended');
 await assertDetail('sidebar resize without window resize');
 await page.locator('.main-area').evaluate(element=>{element.style.marginLeft='';});
 await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
 for(const viewport of [{width:1180,height:650},{width:1000,height:720},{width:600,height:620},{width:390,height:560}]){
  await page.setViewportSize(viewport);await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  await page.evaluate(()=>window.scrollTo(0,document.body.scrollHeight));await assertDetail(`${viewport.width}x${viewport.height}`);
  const metrics=await page.evaluate(()=>({document:document.documentElement.scrollWidth,viewport:innerWidth}));assert(metrics.document<=metrics.viewport+1,'Responsive page horizontally overflows');
 }
 await page.locator('[data-seeds-action="close-detail"]').click();assert.equal(await page.locator('.seed-detail').count(),0);assert.equal(await page.locator('[data-seed-id="1"]').evaluate(element=>element===document.activeElement),true,'Closing details returns keyboard focus without losing the seed');
 assert.deepEqual(blocked,[],'External network attempted');assert.deepEqual(errors,[],'Browser runtime errors');
 const calls=await page.evaluate(()=>window.__seedLayoutCalls);assert(calls.every(([command])=>['list_project_seeds','read_project_seed'].includes(command)),'Layout interactions performed a mutation');
 const observed=await page.evaluate(()=>({observers:window.__seedLayoutObservers,listeners:window.__seedLayoutListenerCount()}));
 assert(observed.observers.some(observer=>observer.observed.includes('topbar')&&observer.observed.includes('workspace-section-navigation')),'Actual shell chrome is not observed for independent width changes');
 assert(observed.listeners>0,'Layout scroll/resize handlers were not installed');
 await page.evaluate(()=>window.__seedLayoutUnmount());
 const disposed=await page.evaluate(()=>({observers:window.__seedLayoutObservers,listeners:window.__seedLayoutListenerCount()}));
 assert(disposed.observers.every(observer=>observer.disconnected),'ResizeObserver was retained after leaving the seed workspace');assert.equal(disposed.listeners,0,'Scroll/resize handler was retained after leaving the seed workspace');
 console.log(JSON.stringify({passed:results.length,results,cleanupVerified:true,readOnly:true,externalNetwork:false},null,2));
}finally{await browser?.close();await server.close();}
