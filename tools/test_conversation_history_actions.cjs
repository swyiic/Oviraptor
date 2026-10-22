const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vue=require('vue');
const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc');
const {loadPure,transpile}=require('./shell_module_harness.cjs');
const filename='src/features/agentHost/components/ConversationHistoryList.vue';
const {descriptor}=parse(fs.readFileSync(filename,'utf8'),{filename});
const script=compileScript(descriptor,{id:'conversation-history-actions'});
const template=compileTemplate({source:descriptor.template.content,filename,id:'conversation-history-actions',compilerOptions:{bindingMetadata:script.bindings}});
const flush=async()=>{for(let i=0;i<3;i++)await new Promise(setImmediate);};
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no});return{promise,resolve,reject};};
const conversation=(id,patch={})=>({id,projectId:1,title:`Title ${id}`,createdAt:'2026-10-20T09:00:00.000Z',lastActivityAt:'2026-10-23T04:00:00.000Z',pinned:false,...patch});
function nodes(value,result=[]){if(Array.isArray(value))for(const x of value)nodes(x,result);else if(value&&typeof value==='object'){result.push(value);nodes(value.children,result);}return result;}
function text(value){if(Array.isArray(value))return value.map(text).join('');return typeof value==='string'?value:value&&typeof value==='object'?text(value.children):'';}
function mount(t,overrides={}){
 const callbacks=[],calls=[],emits=[],exports={};
 new Function('require','exports',transpile(script.content))(name=>{
  if(name==='vue')return{...vue,onBeforeUnmount:fn=>callbacks.push(fn)};
  if(name==='@lucide/vue')return new Proxy({},{get:()=>({render:()=>null})});
  if(name.endsWith('/i18n'))return{useI18n:()=>({tr:zh=>zh,locale:vue.ref('zh')})};
  if(name==='../conversationHistory')return loadPure('src/features/agentHost/conversationHistory.ts');
  throw Error(name);
 },exports);
 const rendered={};new Function('require','exports',transpile(template.code))(name=>name==='vue'?vue:require(name),rendered);
 const props=vue.reactive({items:[conversation('first'),conversation('selected',{pinned:true})],selectedId:'selected',projectId:1,emptyLabel:'Empty',updateConversation:async(...args)=>{calls.push(['update',...args]);},revealConversation:async(...args)=>{calls.push(['reveal',...args]);return '/private/shared workspace';},removeConversation:async(...args)=>{calls.push(['remove',...args]);return true;},...overrides});
 const scope=vue.effectScope(),b=scope.run(()=>exports.default.setup(props,{expose(){},emit(...args){emits.push(args)}}));
 t.after(()=>{callbacks.forEach(fn=>fn());scope.stop()});
 const all=()=>nodes(rendered.render({},[],props,vue.proxyRefs(b),{},{}));
 return{b,props,calls,emits,all,click:async label=>{const button=all().find(v=>v.type==='button'&&text(v)===label);assert.ok(button,`button ${label} rendered`);await button.props.onClick({});await flush();}};
}
const rightClick=()=>({type:'contextmenu',clientX:99999,clientY:99999,preventDefault(){},stopPropagation(){}});
test('history rows group durable pins, show real last activity and open the clicked row menu without selecting it',async t=>{
 const h=mount(t);assert.deepEqual(template.errors,[]);
 assert.deepEqual(h.b.groups.value.map(g=>[g.label,g.items.map(x=>x.id)]),[['置顶',['selected']],['最近对话',['first']]]);
 assert.equal(h.all().filter(v=>v.type==='time').length,2);
 const row=h.all().find(v=>v.props?.class?.includes?.('chat-history-row')&&text(v).includes('Title first'));
 assert.ok(row);await row.props.onContextmenu(rightClick());await flush();
 assert.equal(h.b.menu.value.id,'first');assert.deepEqual(h.emits,[]);
 assert.ok(h.b.menu.value.x<=1024-232);assert.ok(h.b.menu.value.y<=768-205);
 assert.deepEqual(h.all().filter(v=>v.props?.role==='menuitem').map(text),['重命名','置顶','在 Finder 中显示工作区','删除对话…']);
});
test('rename Enter saves its original row, keeps the draft on failure and Escape cancels without writing',async t=>{
 let attempts=0;const h=mount(t,{updateConversation:async(...args)=>{h.calls.push(['update',...args]);if(!attempts++)throw Error('disk_busy')}});
 await h.b.openMenu(rightClick(),h.props.items[0]);await h.click('重命名');h.b.titleDraft.value='我的证书分析';
 const submit=()=>h.all().find(v=>v.type==='form').props.onSubmit({preventDefault(){}});
 await submit();assert.equal(h.b.dialog.value.id,'first');assert.equal(h.b.titleDraft.value,'我的证书分析');assert.match(h.b.notice.value,/disk_busy/);
 await submit();assert.equal(h.b.dialog.value,undefined);assert.deepEqual(h.calls,[['update','first',{title:'我的证书分析'}],['update','first',{title:'我的证书分析'}]]);
 await h.b.openMenu(rightClick(),h.props.items[0]);await h.click('重命名');h.b.titleDraft.value='cancelled';h.b.dialogKey({key:'Escape',preventDefault(){}});assert.equal(h.b.dialog.value,undefined);assert.equal(h.calls.length,2);
});
test('pin and reveal target the context row; reveal states the shared workspace path and cannot delete without confirmation',async t=>{
 const h=mount(t);await h.b.openMenu(rightClick(),h.props.items[0]);await h.click('置顶');assert.deepEqual(h.calls[0],['update','first',{pinned:true}]);
 await h.b.openMenu(rightClick(),h.props.items[1]);await h.click('取消置顶');assert.deepEqual(h.calls[1],['update','selected',{pinned:false}]);
 await h.b.openMenu(rightClick(),h.props.items[0]);await h.click('在 Finder 中显示工作区');assert.deepEqual(h.calls[2],['reveal','first']);assert.match(h.b.notice.value,/项目共享工作区.*private\/shared workspace/);
 await h.b.openMenu(rightClick(),h.props.items[0]);await h.click('删除对话…');assert.equal(h.calls.length,3);assert.equal(h.b.dialog.value.kind,'delete');h.b.closeDialog();assert.equal(h.calls.length,3);
 await h.b.openMenu(rightClick(),h.props.items[0]);await h.click('删除对话…');await h.b.saveDialog();assert.deepEqual(h.calls[3],['remove','first']);
});
test('project changes discard open editors and late metadata results cannot reopen or overwrite a new project notice',async t=>{
 const hold=deferred(),h=mount(t,{updateConversation:()=>hold.promise});await h.b.openMenu(rightClick(),h.props.items[0]);await h.click('重命名');h.b.titleDraft.value='old';const saving=h.b.saveDialog();
 assert.equal(h.b.saving.value,true);h.props.projectId=2;h.props.items=[conversation('new',{projectId:2})];await flush();assert.equal(h.b.dialog.value,undefined);assert.equal(h.b.saving.value,false);
 hold.reject(Error('late failure'));await saving;assert.equal(h.b.notice.value,'');assert.equal(h.b.dialog.value,undefined);
});
test('metadata actions suppress double submission and delete stays disabled during an active operation',async t=>{
 const hold=deferred(),h=mount(t,{updateConversation:(...args)=>{h.calls.push(args);return hold.promise},deletingDisabled:true});
 await h.b.openMenu(rightClick(),h.props.items[0]);await h.b.openDialog('delete');assert.equal(h.b.dialog.value,undefined);
 const pinned=h.b.togglePin();await h.b.togglePin();assert.equal(h.calls.length,1);hold.resolve();await pinned;
 await h.b.openMenu(rightClick(),h.props.items[0]);await h.b.openDialog('rename');h.b.titleDraft.value='😃'.repeat(65);await h.b.saveDialog();assert.match(h.b.notice.value,/256/);assert.equal(h.calls.length,1);
});
test('history sorting uses pin then activity while preserving newest creation order for old rows without timestamps',()=>{
 const {sortConversationHistory,conversationHistoryTime}=loadPure('src/features/agentHost/conversationHistory.ts');
 const list=[conversation('old',{lastActivityAt:'2026-10-20T00:00:00Z'}),conversation('new'),conversation('pin',{pinned:true,lastActivityAt:'2020-01-01T00:00:00Z'})];assert.deepEqual(sortConversationHistory(list).map(x=>x.id),['pin','new','old']);assert.equal(list[0].id,'old');
 assert.deepEqual(sortConversationHistory([{id:'1'},{id:'2'}]).map(x=>x.id),['2','1']);
 assert.deepEqual(conversationHistoryTime('invalid'),{short:'',full:''});const yesterday=new Date(2026,9,22,10,20);assert.match(conversationHistoryTime(yesterday.toISOString(),'zh',new Date(2026,9,23,12)).short,/昨天/);
});
