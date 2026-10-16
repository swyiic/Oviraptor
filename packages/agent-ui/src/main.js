import {HostApi,entityId,segment} from './api.js';
import {ChatController} from './controller.js';
import {appPageUrl,bridgeMessage,BRIDGE_PROTOCOL,sessionToken} from './security.js';
import {node,button} from './dom.js';
import {createSettingsPanel,createSkillsPanel} from './panels.js';
import {snapshotDisclosure,matchesDisclosure} from './consent.js';
import {UpgradeDraft,upgradeSnapshot} from './upgrades.js';
const $=id=>document.getElementById(id);
const api=new HostApi({token:sessionToken(document,window)});
const controller=new ChatController(api,render);
const appUi={open:false,error:'',pending:new Set(),uninstall:null,grants:new Map(),consents:new Map(),page:null,epoch:0,runtimeIdentity:null,observedRuntimeIdentity:null,upgrades:new Map()};
const stopped=new AbortController();let reconnectTimer;
const skillsPanel=createSkillsPanel(api,()=>controller.state.conversationId);
const settingsPanel=createSettingsPanel(api,async()=>{controller.applyStatus(await api.request('/api/status'));appUi.consents.clear();skillsPanel.invalidate();await refreshApps();render();});
const title=node('h1',{},'Oviraptor');
const projectSelect=node('select',{id:'project-select','aria-label':'Project',onChange:event=>{closePage();void controller.selectProject(controller.state.projects.find(p=>entityId(p.id)===event.target.value)?.id);}});
const newProject=button('New project',()=>{$('new-project-form').hidden=!$('new-project-form').hidden;$('project-name').focus();});
const projectForm=node('form',{id:'new-project-form',hidden:true,onSubmit:async event=>{event.preventDefault();const input=$('project-name');if(!input.value.trim())return;closePage();try{await controller.createProject(input.value.trim());input.value='';projectForm.hidden=true;}catch(error){controller.fail(error);}}},node('label',{for:'project-name'},'Project name'),node('input',{id:'project-name',required:true,maxlength:120}),node('button',{type:'submit'},'Create project'));
const newChat=button('New conversation',()=>{closePage();void action(()=>controller.createConversation());},{id:'new-conversation',class:'primary'});
const conversationList=node('nav',{id:'conversations','aria-label':'Conversations'});
const runtime=node('span',{id:'runtime-state',class:'status'}),connection=node('span',{id:'connection-state',class:'status'});
const header=node('header',{class:'topbar'},node('div',{class:'brand'},title,node('small',{},'Agent workspace')),node('div',{class:'topbar-actions'},runtime,connection,button('Skills',()=>skillsPanel.open(),{id:'manage-skills'}),button('Settings',()=>settingsPanel.open(),{id:'manage-settings'}),button('Applications',()=>{appUi.open=true;appUi.consents.clear();renderApps();$('apps-dialog').showModal();void appAction('refresh',async()=>{});},{id:'manage-apps'})));
const sidebar=node('aside',{class:'sidebar'},node('label',{for:'project-select'},'Project'),projectSelect,newProject,projectForm,newChat,conversationList,node('p',{class:'muted small'},'Projects and chat belong to the host. Applications are optional.'));
const mainTitle=node('h2',{id:'conversation-title'},'Your workspace');
const cancel=button('Cancel run',()=>void action(()=>controller.cancel()),{id:'cancel-run',class:'danger',hidden:true});
const globalError=node('p',{id:'error',class:'error',role:'alert',hidden:true}),notice=node('p',{id:'notice',class:'notice',role:'status',hidden:true});
const runtimeNotice=node('p',{id:'runtime-notice',class:'runtime-notice'});
const messages=node('div',{id:'messages',class:'messages',role:'log','aria-label':'Conversation messages','aria-live':'polite'});
const approvals=node('section',{id:'approvals','aria-label':'Runtime permission requests'});
const draft=node('textarea',{id:'message-input',rows:3,maxlength:16384,'aria-label':'Message',placeholder:'Write a message…',onInput:event=>{controller.setDraft(event.target.value);updateComposer();},onKeydown:event=>{if(event.key==='Enter'&&(event.ctrlKey||event.metaKey)){event.preventDefault();void controller.send();}}});
const send=node('button',{id:'send-message',type:'submit',class:'primary'},'Send message');
const composer=node('form',{id:'composer',class:'composer',onSubmit:event=>{event.preventDefault();void controller.send();}},draft,node('div',{class:'composer-footer'},node('small',{class:'muted'},'Ctrl / ⌘ + Enter · Local conversation storage'),send));
const chat=node('section',{id:'chat-view',class:'chat-view'},node('header',{class:'chat-header'},mainTitle,cancel),runtimeNotice,messages,approvals,composer);
const appPage=node('section',{id:'app-page',class:'app-page',hidden:true});
const main=node('main',{},globalError,notice,chat,appPage);
const appDialog=node('dialog',{id:'apps-dialog',class:'apps-dialog',onClose:()=>{appUi.open=false;}},node('header',{class:'dialog-heading'},node('div',{},node('h2',{},'Applications'),node('p',{class:'muted'},'Install a local application package. Chat works with no applications installed.')),button('Close',()=>appDialog.close(),{'aria-label':'Close applications'})),node('form',{id:'install-form',class:'install-form',onSubmit:async event=>{event.preventDefault();const input=$('install-path');if(!input.value.trim())return;await appAction('install',async()=>{await api.post('/api/apps',{path:input.value.trim()});input.value='';});}},node('label',{for:'install-path'},'Local package directory'),node('input',{id:'install-path',required:true,placeholder:'/path/to/application',autocomplete:'off'}),node('button',{type:'submit',class:'primary',id:'install-app'},'Install application')),node('p',{id:'app-error',class:'error',role:'alert',hidden:true}),node('div',{id:'apps-list',class:'apps-list'}),button('Refresh applications',()=>void appAction('refresh',refreshApps)));
$('app').append(header,node('div',{class:'workspace'},sidebar,main),appDialog,skillsPanel.dialog,settingsPanel.dialog);
async function action(fn){try{await fn();}catch(error){controller.fail(error);}}
async function refreshApps(){appUi.consents.clear();for(const draft of appUi.upgrades.values())draft.invalidate();const [apps,status]=await Promise.all([api.list('/api/apps'),api.request('/api/status')]);controller.state.apps=apps;controller.applyStatus(status);appUi.runtimeIdentity=typeof status.runtime?.identity==='string'?status.runtime.identity:null;render();}
async function appAction(key,fn){if(appUi.pending.has(key))return;appUi.pending.add(key);appUi.error='';renderApps();try{await fn();appUi.consents.clear();for(const draftKey of appUi.grants.keys())if(draftKey.startsWith(key+':')||key==='refresh')appUi.grants.delete(draftKey);await refreshApps();}catch(error){appUi.error=error.message||String(error);appUi.consents.clear();if(error.code==='host_runtime_identity_changed')appUi.runtimeIdentity=null;}finally{appUi.pending.delete(key);renderApps();}}
function updateComposer(){const s=controller.state,key=entityId(s.conversationId),busy=controller.pendingWrites.has(key)||s.runs.has(key);draft.disabled=s.conversationId===null||controller.pendingWrites.has(key);send.disabled=s.conversationId===null||busy||!controller.draft().trim();send.textContent=controller.pendingWrites.has(key)?'Saving…':s.runtime.state==='unconfigured'?'Save message':'Send message';}
function render(){
  skillsPanel.syncContext(controller.state.conversationId);
  const s=controller.state;if(appUi.observedRuntimeIdentity!==s.runtime.identity){appUi.consents.clear();appUi.observedRuntimeIdentity=s.runtime.identity;if(appUi.runtimeIdentity!==s.runtime.identity)appUi.runtimeIdentity=null;}
  const focused=document.activeElement?.id,start=document.activeElement?.selectionStart,end=document.activeElement?.selectionEnd;
  runtime.textContent=`Runtime · ${s.runtime.state||'unknown'}`;runtime.dataset.state=s.runtime.state||'unknown';connection.textContent=`Events · ${s.connection}`;
  const currentProject=entityId(s.projectId);projectSelect.replaceChildren(...s.projects.map(p=>{const option=node('option',{value:entityId(p.id)},p.name||p.title||entityId(p.id));option.selected=entityId(p.id)===currentProject;return option;}));projectSelect.disabled=!s.projects.length;newChat.disabled=s.projectId===null;
  conversationList.replaceChildren(...s.conversations.map(c=>button(c.title||'Conversation',()=>{closePage();void controller.selectConversation(c.id);},{class:entityId(c.id)===entityId(s.conversationId)?'conversation selected':'conversation','aria-current':entityId(c.id)===entityId(s.conversationId)?'page':null,'data-conversation-id':entityId(c.id)})));
  mainTitle.textContent=s.conversations.find(c=>entityId(c.id)===entityId(s.conversationId))?.title||(!s.projects.length?'Create your first project':!s.conversations.length?'Start a conversation':'Conversation');
  globalError.textContent=s.error;globalError.hidden=!s.error;notice.textContent=s.notice;notice.hidden=!s.notice;
  runtimeNotice.textContent=s.runtime.state==='unconfigured'?'No runtime is configured. Messages can be saved locally; no model connection is made.':s.runtime.state==='failed'?'The runtime is unavailable. Review the host configuration; requests are not resent automatically.':s.runtime.message||'Application tools require their own declared project permissions.';
  const nearEnd=messages.scrollHeight-messages.scrollTop-messages.clientHeight<90;
  messages.replaceChildren(...s.messages.map(m=>node('article',{class:`message ${m.role==='assistant'?'assistant':'user'}`,'data-message-id':entityId(m.id)},node('strong',{},m.role==='assistant'?'Assistant':m.role==='system'?'System':'You'),node('div',{class:'message-text'},String(m.text??m.content??'')),m.outcomeUnknown===true?node('small',{class:'error'},'结果未知，未自动重试 · Outcome unknown'):m.streaming?node('small',{class:'muted'},'Receiving…'):['failed','cancelled','interrupted','refused','incomplete'].includes(m.status)?node('small',{class:'error'},`Run ${m.status} · no automatic retry`):null)));
  if(!s.messages.length)messages.append(node('div',{class:'welcome'},node('h3',{},s.loading?'Reading conversation…':'What would you like to work on?'),node('p',{},s.loading?'Waiting for the host.':'This chat has no dependency on an installed business application.')));
  if(nearEnd)messages.scrollTop=messages.scrollHeight;
  const currentApprovals=s.approvals.filter(p=>entityId(p.conversationId)===entityId(s.conversationId));approvals.replaceChildren(...currentApprovals.map(p=>{const body=node('article',{class:'approval','data-permission-id':p.id},node('h3',{},p.title||'Runtime permission requested'),node('p',{},p.description||'Review the exact request before choosing an option.'),node('small',{class:'muted'},`Run ${p.runId||'unknown'}`));for(const option of p.options||[])body.append(button(option.label||option.name||String(option.id),()=>void action(()=>controller.approve(p.id,option.id??option.optionId)),{'data-permission-option':option.id??option.optionId}));if(!p.options?.length)body.append(node('p',{class:'error'},'No valid approval options were supplied. Nothing has been authorized.'));return body;}));
  const run=s.runs.get(entityId(s.conversationId));cancel.hidden=!run;cancel.disabled=run?.state==='cancelling';
  draft.value=controller.draft();updateComposer();
  if(appUi.open)renderApps();
  if(focused&&$(focused)){const element=$(focused);element.focus({preventScroll:true});if(typeof start==='number'&&element.setSelectionRange)element.setSelectionRange(start,end);}
}
function permissions(app){const values=app.permissions||app.manifest?.permissions||[];return values.map(p=>typeof p==='string'?{id:p,description:p}:p).filter(p=>typeof p.id==='string');}
function renderApps(){
  const s=controller.state;$('app-error').textContent=appUi.error;$('app-error').hidden=!appUi.error;$('install-app').disabled=appUi.pending.has('install');
  const cards=s.apps.map(app=>{
    const id=String(app.id),pending=appUi.pending.has(id),busy=Boolean(app.busy),card=node('article',{class:'application','data-app-id':id});
    card.append(node('header',{},node('div',{},node('h3',{},app.name||id),node('small',{class:'muted'},`${id} · ${app.version||'version unknown'}`)),node('span',{class:'status'},app.enabled?'Enabled':'Disabled')));
    if(app.busy)card.append(node('p',{class:'notice'},'Application busy. Uninstall is refused until accepted work settles. Disable immediately stops new admission; accepted background work continues.'));
    const actions=node('div',{class:'actions'},button(app.enabled?'Disable':'Enable',()=>void appAction(id,()=>api.post(`/api/apps/${segment(id)}/state`,{enabled:!app.enabled})),{disabled:pending,'data-app-action':'toggle'}),button('Uninstall',()=>{appUi.uninstall=id;renderApps();},{disabled:pending||busy,'data-app-action':'uninstall'}));
    for(const page of app.pages||[])actions.append(button(page.title||page.id,()=>{try{openPage(app,page);appDialog.close();}catch(error){appUi.error=error.message;renderApps();}},{disabled:pending||!app.enabled,'data-app-page':page.id}));card.append(actions);
    if(appUi.uninstall===id)card.append(node('div',{class:'uninstall-confirm',role:'alert'},node('p',{},'Uninstall this application? Application data will be preserved. This does not delete projects or conversations.'),button('Cancel',()=>{appUi.uninstall=null;renderApps();}),button('Uninstall · keep data',()=>void appAction(id,async()=>{await api.delete(`/api/apps/${segment(id)}`);appUi.uninstall=null;if(appUi.page?.appId===id)closePage();}),{disabled:pending||busy,class:'danger','data-app-action':'confirm-uninstall'})));
    if(!appUi.upgrades.has(id))appUi.upgrades.set(id,new UpgradeDraft());
    const upgrade=appUi.upgrades.get(id),upgradeState=upgrade.state,inputId=`upgrade-path-${id}`;
    const upgradeInput=node('input',{id:inputId,required:true,autocomplete:'off',disabled:pending,value:upgradeState.path,onInput:event=>{upgrade.setPath(event.target.value);render();}});
    const upgradeConsent=node('input',{type:'checkbox',disabled:pending||!upgradeSnapshot(app,upgradeState.path),onChange:event=>{upgrade.confirm(app,event.target.checked);renderApps();},'data-upgrade-confirm':id});upgradeConsent.checked=upgrade.ready(app);
    const upgradeForm=node('form',{class:'upgrade-form',onSubmit:event=>{event.preventDefault();if(!upgrade.ready(app))return;void appAction(id,async()=>{const result=await upgrade.run(api,app);if(result.items)controller.state.apps=result.items;if(!result.ok)throw Error(upgrade.state.error||'Upgrade state could not be verified.');if(appUi.page?.appId===id)closePage();});}},node('p',{class:'muted small'},`Observed ${app.version} · revision ${app.revision??'unknown'}. Upgrade only after explicitly disabling this application and waiting for accepted work to settle.`),node('label',{for:inputId},'New local package directory'),upgradeInput,node('label',{class:'skill-choice'},upgradeConsent,node('span',{},'Confirm upgrade from this directory at the displayed revision. The application stays disabled and permissions require review.')),node('button',{type:'submit',class:'primary',disabled:pending||!upgrade.ready(app),'data-upgrade-submit':id},'Upgrade application'),upgradeState.error?node('p',{class:'error',role:'alert'},upgradeState.error):null,upgradeState.notice?node('p',{class:'notice',role:'status'},upgradeState.notice):null);
    card.append(node('details',{class:'application-upgrade',open:upgradeState.open,onToggle:event=>{upgradeState.open=event.target.open;}},node('summary',{},'Upgrade from local package'),upgradeForm));
    const declared=permissions(app),projectId=s.projectId,key=`${id}:${entityId(projectId)}`;
    const runtimeIdentity=appUi.runtimeIdentity;
    const existing=(app.grants||[]).find(g=>entityId(g.projectId)===entityId(projectId));
    if(!appUi.grants.has(key))appUi.grants.set(key,new Set(existing?.permissions||[]));
    const chosen=appUi.grants.get(key),grantForm=node('form',{class:'grant-form',onSubmit:event=>{event.preventDefault();if(!matchesDisclosure(appUi.consents.get(key),runtimeIdentity,existing?.revision??0))return;void appAction(id,()=>api.post(`/api/apps/${segment(id)}/grants`,{projectId,permissions:[...chosen],expectedRevision:appUi.consents.get(key)?.grantRevision,expectedRuntimeIdentity:appUi.consents.get(key)?.runtimeIdentity,disclosureAccepted:appUi.consents.get(key)?.accepted===true}));}},node('h4',{},'Project permissions'),node('p',{class:'muted'},projectId===null?'Choose a project first.':`Current project only · saved revision ${existing?.revision??0}`),node('p',{class:'muted small'},`Runtime recipient: ${runtimeIdentity||'unknown · refresh before granting permissions'}`));
    for(const permission of declared){const input=node('input',{type:'checkbox',disabled:pending||!app.enabled||projectId===null,onChange:event=>{if(event.target.checked)chosen.add(permission.id);else chosen.delete(permission.id);appUi.consents.set(key,false);renderApps();}});input.checked=chosen.has(permission.id);grantForm.append(node('label',{},input,node('span',{},permission.id,node('small',{class:'muted'},permission.description||''))));}
    const consent=node('input',{type:'checkbox',disabled:pending||!app.enabled||projectId===null||!runtimeIdentity,onChange:event=>{appUi.consents.set(key,event.target.checked?snapshotDisclosure(runtimeIdentity,existing?.revision??0):null);renderApps();},'data-grant-consent':id});consent.checked=matchesDisclosure(appUi.consents.get(key),runtimeIdentity,existing?.revision??0);
    grantForm.append(node('label',{class:'grant-disclosure'},consent,node('span',{},"允许该项目的 Agent 使用所选应用权限，并将工具数据发送给当前运行时。",node('small',{class:'muted'},"Allow this project's Agent to use the selected application permissions and send tool data to the current runtime. Installing or opening an application does not grant permission."))));
    grantForm.append(node('button',{type:'submit',disabled:pending||!app.enabled||projectId===null||!declared.length||!matchesDisclosure(appUi.consents.get(key),runtimeIdentity,existing?.revision??0)},'Save project permissions'));if(!declared.length)grantForm.append(node('p',{class:'muted'},'This package declares no grantable permissions.'));card.append(grantForm);return card;
  });
  $('apps-list').replaceChildren(...(cards.length?cards:[node('p',{class:'empty-apps'},'No applications installed. Projects and chat remain available.')]));
}
function closePage(){appUi.page=null;appUi.epoch++;appPage.replaceChildren();appPage.hidden=true;chat.hidden=false;}
function openPage(app,page){
  const url=appPageUrl(page.url,app.id,location.origin),nonce=crypto.randomUUID(),epoch=++appUi.epoch;
  const frame=node('iframe',{src:url,sandbox:'allow-scripts',title:`${app.name||app.id}: ${page.title||page.id}`,referrerpolicy:'no-referrer','data-app-frame':app.id});
  const close=button('Close page · return to Chat',closePage);appUi.page={appId:app.id,nonce,frame,epoch};
  frame.addEventListener('load',()=>{if(appUi.page?.epoch===epoch)frame.contentWindow.postMessage({protocol:BRIDGE_PROTOCOL,type:'init',appId:app.id,projectId:controller.state.projectId,nonce},'*');});
  appPage.replaceChildren(node('header',{class:'chat-header'},node('h2',{},page.title||app.name||app.id),close),node('p',{class:'muted small'},'Application page · isolated frame. Closing this page does not disable the application.'),frame);chat.hidden=true;appPage.hidden=false;
}
window.addEventListener('message',event=>{const current=appUi.page;if(!current)return;const message=bridgeMessage(event,{source:current.frame.contentWindow,appId:current.appId,nonce:current.nonce});if(!message)return;if(message.type==='close')closePage();else if(message.type==='resize')current.frame.style.height=`${message.height}px`;else if(message.type==='notice'){controller.state.notice=message.text;render();}});
async function connectEvents(){
  let delay=1000;
  while(!stopped.signal.aborted){try{await api.events(event=>{if((event.type==='runtime.status'&&!entityId(event.conversationId))||event.type==='host.snapshot'){appUi.consents.clear();skillsPanel.invalidate();}controller.event(event);},{signal:stopped.signal,onState:value=>{controller.state.connection=value;render();}});if(stopped.signal.aborted)break;controller.state.connection='disconnected';render();}catch(error){if(stopped.signal.aborted)break;controller.state.connection='disconnected';controller.state.error=`Event connection unavailable: ${error.message}. Messages are not resent automatically.`;render();}
    await new Promise(resolve=>{reconnectTimer=setTimeout(resolve,delay);stopped.signal.addEventListener('abort',resolve,{once:true});});delay=Math.min(delay*2,10000);
  }
}
window.addEventListener('pagehide',()=>{stopped.abort();clearTimeout(reconnectTimer);controller.dispose();},{once:true});
render();void controller.init();void connectEvents();
