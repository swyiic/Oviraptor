const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs');
const vue=require('vue'),{renderToString}=require('@vue/server-renderer');
const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc');
const {catalog,transpile}=require('./shell_module_harness.cjs');
async function render(file,props){
 const {descriptor}=parse(fs.readFileSync(file,'utf8'),{filename:file});
 const script=compileScript(descriptor,{id:'focused-navigation'}),scriptExports={};
 const req=name=>{if(name==='vue')return vue;if(name==='@lucide/vue')return new Proxy({},{get:()=>({render:()=>null})});if(name.endsWith('/i18n'))return{useI18n:()=>({tr:(_,en)=>en})};if(name==='./moduleCatalog')return catalog;throw Error(name);};
 new Function('require','exports',transpile(script.content))(req,scriptExports);
 const output=compileTemplate({source:descriptor.template.content,filename:file,id:'focused-navigation',compilerOptions:{bindingMetadata:script.bindings}}),compiled={};
 assert.deepEqual(output.errors,[]);new Function('require','exports',transpile(output.code))(req,compiled);
 const component=scriptExports.default,setup=component.setup;let bindings;component.setup=(p,c)=>{bindings=setup(p,c);return bindings;};component.render=compiled.render;
 const context={};const html=await renderToString(vue.createSSRApp(component,props),context);
 return{html,teleport:context.teleports?.['.app-shell']||'',bindings};
}
const section='src/shell/WorkspaceSectionNavigation.vue',finder='src/features/conversations/ConversationFinder.vue';
test('asset sections keep one active page without duplicating the global logs entry',async()=>{
 const actions=[];const h=await render(section,{module:'asset',view:'assets',sentinelMenu:'dialog',blockedCount:29183,onNavigate:item=>actions.push(item)});
 for(const item of catalog.workspaceNavigation('asset'))assert.match(h.html,new RegExp(`<option value="${item.key}"`));
 assert.doesNotMatch(h.html,/<option value="logs"/);assert.match(h.html,/Asset list/);assert.match(h.html,/>29,183<\/small>/);assert.match(h.html,/<option value="quarantine">[^<]*29,183<\/option>/);assert.doesNotMatch(h.html,/999\+/);
 assert.equal((h.html.match(/aria-current="page"/g)||[]).length,1);
 h.bindings.choose({target:{value:'quarantine'}});assert.equal(actions[0].view,'quarantine');
});
test('quarantine displays exact zero and large counts in desktop and compact navigation',async()=>{
 for(const count of [0,1234567]){
  const h=await render(section,{module:'asset',view:'quarantine',sentinelMenu:'dialog',blockedCount:count});
  const text=count.toLocaleString('en-US').replace(/[.*+?^${}()|[\]\\]/g,'\\$&');
  assert.match(h.html,new RegExp(`>${text}</small>`));
  assert.match(h.html,new RegExp(`<option value="quarantine"[^>]*>[^<]* · ${text}</option>`));
  assert.doesNotMatch(h.html,/999\+/);
 }
 const unavailable=await render(section,{module:'asset',view:'assets',sentinelMenu:'dialog'});
 assert.doesNotMatch(unavailable.html,/<small/);
});
test('scan sections keep their section identity instead of losing it to the shared sentinel route',async()=>{
 const actions=[];const h=await render(section,{module:'sentinel',view:'sentinel',sentinelMenu:'queue',fuse:3,onNavigate:item=>actions.push(item)});
 assert.equal((h.html.match(/aria-current="page"/g)||[]).length,1);
 h.bindings.choose({target:{value:'urls'}});assert.equal(actions[0].key,'urls');assert.equal(actions[0].section,'results');
 h.bindings.choose({target:{value:'unknown'}});assert.equal(actions.length,1);
});
test('finder searches the entire supplied history while bounding rendered results and never emitting mutations',async()=>{
 const rows=Array.from({length:500},(_,i)=>({id:String(i),title:`Saved certificate host-${i}.example.test`}));const events=[];
 const h=await render(finder,{open:true,query:'host-499',items:rows,projectName:'Current project',onSelect:id=>events.push(['select',id]),'onUpdate:open':v=>events.push(['open',v])});
 assert.match(h.teleport,/host-499/);assert.doesNotMatch(h.teleport,/host-498/);assert.match(h.teleport,/Current project/);
 h.bindings.choose('499');assert.deepEqual(events,[['select','499'],['open',false]]);
 const many=await render(finder,{open:true,query:'',items:rows});assert.equal((many.teleport.match(/role="option"/g)||[]).length,60);assert.match(many.teleport,/Show more conversations/);
});
test('finder supports keyboard selection, IME Enter guard and blocked selection',async()=>{
 const selected=[];const h=await render(finder,{open:true,query:'',items:[{id:'a',title:'First'},{id:'b',title:'Second'}],onSelect:id=>selected.push(id)});
 h.bindings.input.value={};const e=key=>({key,target:h.bindings.input.value,preventDefault(){},stopPropagation(){}});
 h.bindings.handleKey(e('ArrowDown'));h.bindings.handleKey({...e('Enter'),isComposing:true});assert.equal(selected.length,0);
 h.bindings.handleKey(e('Enter'));assert.deepEqual(selected,['b']);
 const blocked=await render(finder,{open:true,query:'',items:[{id:'x',title:'Protected'}],disabled:true,onSelect:id=>selected.push(id)});blocked.bindings.choose('x');assert.deepEqual(selected,['b']);
});
