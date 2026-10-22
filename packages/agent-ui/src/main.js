import {HostApi,entityId,segment} from './api.js';
import {ChatController} from './controller.js';
import {appPageUrl,bridgeMessage,BRIDGE_PROTOCOL,NATIVE_BRIDGE_PROTOCOL,eventInvalidatesPage,sessionToken} from './security.js';
import {node,button} from './dom.js';
import {createRefreshFeedback} from './refresh-feedback.js';
import {createSettingsPanel,createSkillsPanel} from './panels.js';
import {snapshotDisclosure,matchesDisclosure} from './consent.js';
import {UpgradeDraft,upgradeSnapshot} from './upgrades.js';
import {pairedSkillsIncludeBody,pairedSkillsSummary} from './paired-skills.js';
import {pageScope,reviewNavigation} from './navigation.js';
const $=id=>document.getElementById(id);
const api=new HostApi({token:sessionToken(document,window)});
const controller=new ChatController(api,render);
const appUi={open:false,error:'',pending:new Set(),uninstall:null,grants:new Map(),consents:new Map(),page:null,epoch:0,runtimeIdentity:null,observedRuntimeIdentity:null,upgrades:new Map(),pairedPreviews:new Map(),pairedNotice:''};
const stopped=new AbortController();let reconnectTimer;
const skillsPanel=createSkillsPanel(api,()=>controller.state.conversationId);
const settingsPanel=createSettingsPanel(api,async()=>{controller.applyStatus(await api.request('/api/status'));appUi.consents.clear();skillsPanel.invalidate();await refreshApps();render();});
const title=node('h1',{},'Oviraptor');
const projectSelect=node('select',{id:'project-select','aria-label':'Project',onChange:event=>{closePage();void controller.selectProject(controller.state.projects.find(p=>entityId(p.id)===event.target.value)?.id);}});
const newProject=button('New project',()=>{$('new-project-form').hidden=!$('new-project-form').hidden;$('project-name').focus();});
const projectForm=node('form',{id:'new-project-form',hidden:true,onSubmit:async event=>{event.preventDefault();const input=$('project-name');if(!input.value.trim())return;closePage();try{await controller.createProject(input.value.trim());input.value='';projectForm.hidden=true;}catch(error){controller.fail(error);}}},node('label',{for:'project-name'},'Project name'),node('input',{id:'project-name',required:true,maxlength:120}),node('button',{type:'submit'},'Create project'));
const newChat=button('New conversation',()=>{closePage();void action(()=>controller.createConversation());},{id:'new-conversation',class:'primary'});
const startScratch=button('Start chatting',()=>{closePage();void action(()=>controller.startScratchConversation());},{id:'start-scratch',class:'primary'});
const conversationList=node('nav',{id:'conversations','aria-label':'Conversations'});
const promoteForm=node('form',{id:'promote-project-form',class:'promote-form',hidden:true,onSubmit:async event=>{event.preventDefault();const input=$('promote-project-name');if(!input.value.trim())return;closePage();try{await controller.promoteCurrentProject(input.value.trim());input.value='';controller.state.notice='Project saved. Scratch workspace is now a durable project.';}catch(error){controller.fail(error);}}},node('p',{class:'muted small'},'This is a scratch workspace. Name it to keep it as a real project.'),node('label',{for:'promote-project-name'},'Project name'),node('div',{class:'promote-row'},node('input',{id:'promote-project-name',required:true,maxlength:120,placeholder:'e.g. Asset review'}),node('button',{type:'submit',class:'primary'},'Save as project')));
const runtime=node('span',{id:'runtime-state',class:'status'}),connection=node('span',{id:'connection-state',class:'status'});
const header=node('header',{class:'topbar'},node('div',{class:'brand'},title,node('small',{},'Agent workspace')),node('div',{class:'topbar-actions'},runtime,connection,button('Skills',()=>skillsPanel.open(),{id:'manage-skills'}),button('Settings',()=>settingsPanel.open(),{id:'manage-settings'}),button('Applications',()=>{appUi.open=true;appUi.consents.clear();renderApps();$('apps-dialog').showModal();void appAction('refresh',async()=>{});},{id:'manage-apps'})));
const sidebar=node('aside',{class:'sidebar'},node('label',{for:'project-select'},'Project'),projectSelect,newProject,projectForm,startScratch,newChat,conversationList,node('p',{class:'muted small'},'Projects and chat belong to the host. Start chatting creates a scratch workspace you can name later. Applications are optional.'));
const mainTitle=node('h2',{id:'conversation-title'},'Your workspace');
const cancel=button('Cancel run',()=>void action(()=>controller.cancel()),{id:'cancel-run',class:'danger',hidden:true});
const globalError=node('p',{id:'error',class:'error',role:'alert',hidden:true}),notice=node('p',{id:'notice',class:'notice',role:'status',hidden:true});
const runtimeNotice=node('p',{id:'runtime-notice',class:'runtime-notice'});
const messages=node('div',{id:'messages',class:'messages',role:'log','aria-label':'Conversation messages','aria-live':'polite'});
const approvals=node('section',{id:'approvals','aria-label':'Runtime permission requests'});
const insightConfirms=node('section',{id:'insight-confirms','aria-label':'Insight commit confirmations'});
const planProgress=node('section',{id:'plan-progress',class:'plan-progress','aria-label':'Agent plan progress',hidden:true});
const runKeepalive=node('p',{id:'run-keepalive',class:'muted small run-keepalive',hidden:true,role:'status'});
const draft=node('textarea',{id:'message-input',rows:3,maxlength:16384,'aria-label':'Message',placeholder:'Write a message…',onInput:event=>{controller.setDraft(event.target.value);updateComposer();},onKeydown:event=>{if(event.key==='Enter'&&(event.ctrlKey||event.metaKey)){event.preventDefault();void controller.send();}}});
const send=node('button',{id:'send-message',type:'submit',class:'primary'},'Send message');
const composer=node('form',{id:'composer',class:'composer',onSubmit:event=>{event.preventDefault();void controller.send();}},draft,node('div',{class:'composer-footer'},node('small',{class:'muted'},'Ctrl / ⌘ + Enter · Local conversation storage'),send));
const chat=node('section',{id:'chat-view',class:'chat-view'},node('header',{class:'chat-header'},mainTitle,cancel),promoteForm,runtimeNotice,planProgress,runKeepalive,messages,approvals,insightConfirms,composer);
const appPage=node('section',{id:'app-page',class:'app-page',hidden:true});
const main=node('main',{},globalError,notice,chat,appPage);
const applicationRefresh=createRefreshFeedback(()=>appAction('refresh',async()=>{}),{failed:()=>Boolean(appUi.error),disabled:()=>appUi.pending.has('refresh')});
const appDialog=node('dialog',{id:'apps-dialog',class:'apps-dialog',onClose:()=>{appUi.open=false;}},node('header',{class:'dialog-heading'},node('div',{},node('h2',{},'Applications'),node('p',{class:'muted'},'Install a local application package. Chat works with no applications installed.')),button('Close',()=>appDialog.close(),{'aria-label':'Close applications'})),node('form',{id:'install-form',class:'install-form',onSubmit:async event=>{event.preventDefault();const input=$('install-path');if(!input.value.trim())return;await appAction('install',async()=>{const result=await api.post('/api/apps',{path:input.value.trim()});if(result?.id&&result.pairedSkillsPreview)appUi.pairedPreviews.set(String(result.id),result.pairedSkillsPreview);input.value='';});}},node('label',{for:'install-path'},'Local package directory'),node('input',{id:'install-path',required:true,placeholder:'/path/to/application',autocomplete:'off'}),node('button',{type:'submit',class:'primary',id:'install-app'},'Install application')),node('p',{id:'app-error',class:'error',role:'alert',hidden:true}),node('div',{id:'apps-list',class:'apps-list'}),applicationRefresh.button('Refresh applications'));
$('app').append(header,node('div',{class:'workspace'},sidebar,main),appDialog,skillsPanel.dialog,settingsPanel.dialog);
async function action(fn){try{await fn();}catch(error){controller.fail(error);}}
let appsLoadRevision=0;
async function refreshApps(){const requested=++appsLoadRevision;appUi.consents.clear();for(const draft of appUi.upgrades.values())draft.invalidate();const [apps,status]=await Promise.all([api.list('/api/apps'),api.request('/api/status')]);if(requested!==appsLoadRevision)return;controller.state.apps=apps;controller.applyStatus(status);appUi.runtimeIdentity=typeof status.runtime?.identity==='string'?status.runtime.identity:null;render();}
async function appAction(key,fn){if(appUi.pending.has(key))return;appUi.pending.add(key);appUi.error='';renderApps();try{await fn();appUi.consents.clear();for(const draftKey of appUi.grants.keys())if(draftKey.startsWith(key+':')||key==='refresh')appUi.grants.delete(draftKey);await refreshApps();}catch(error){appUi.error=error.message||String(error);appUi.consents.clear();if(error.code==='host_runtime_identity_changed')appUi.runtimeIdentity=null;}finally{appUi.pending.delete(key);renderApps();}}
function updateComposer(){const s=controller.state,key=entityId(s.conversationId),busy=controller.pendingWrites.has(key)||s.runs.has(key);draft.disabled=s.conversationId===null||controller.pendingWrites.has(key);send.disabled=s.conversationId===null||busy||!controller.draft().trim();send.textContent=controller.pendingWrites.has(key)?'Saving…':s.runtime.state==='unconfigured'?'Save message':'Send message';}
function render(){
  skillsPanel.syncContext(controller.state.conversationId);
  const s=controller.state;if(appUi.observedRuntimeIdentity!==s.runtime.identity){appUi.consents.clear();appUi.observedRuntimeIdentity=s.runtime.identity;if(appUi.runtimeIdentity!==s.runtime.identity)appUi.runtimeIdentity=null;}
  const focused=document.activeElement?.id,start=document.activeElement?.selectionStart,end=document.activeElement?.selectionEnd;
  runtime.textContent=`Runtime · ${s.runtime.state||'unknown'}`;runtime.dataset.state=s.runtime.state||'unknown';connection.textContent=`Events · ${s.connection}`;
  const currentProject=entityId(s.projectId);const currentProjectRow=controller.currentProject();projectSelect.replaceChildren(...s.projects.map(p=>{const label=p.kind==='scratch'?`${p.name||p.title||entityId(p.id)} · scratch`:p.name||p.title||entityId(p.id);const option=node('option',{value:entityId(p.id)},label);option.selected=entityId(p.id)===currentProject;return option;}));projectSelect.disabled=!s.projects.length;newChat.disabled=s.projectId===null;startScratch.hidden=s.projectId!==null;newChat.hidden=s.projectId===null;
  conversationList.replaceChildren(...s.conversations.map(c=>button(c.title||'Conversation',()=>{closePage();void controller.selectConversation(c.id);},{class:entityId(c.id)===entityId(s.conversationId)?'conversation selected':'conversation','aria-current':entityId(c.id)===entityId(s.conversationId)?'page':null,'data-conversation-id':entityId(c.id)})));
  mainTitle.textContent=s.conversations.find(c=>entityId(c.id)===entityId(s.conversationId))?.title||(!s.projects.length?'Start chatting, then name a project':!s.conversations.length?'Start a conversation':'Conversation');
  const showPromote=currentProjectRow?.kind==='scratch';promoteForm.hidden=!showPromote;
  globalError.textContent=s.error;globalError.hidden=!s.error;notice.textContent=s.notice;notice.hidden=!s.notice;
  runtimeNotice.textContent=s.runtime.state==='unconfigured'?'No runtime is configured. Messages can be saved locally; no model connection is made.':s.runtime.state==='failed'?'The runtime is unavailable. Review the host configuration; requests are not resent automatically.':s.runtime.message||'Application tools require their own declared project permissions.';
  const nearEnd=messages.scrollHeight-messages.scrollTop-messages.clientHeight<90;
  messages.replaceChildren(...s.messages.map(m=>node('article',{class:`message ${m.role==='assistant'?'assistant':'user'}`,'data-message-id':entityId(m.id)},node('strong',{},m.role==='assistant'?'Assistant':m.role==='system'?'System':'You'),node('div',{class:'message-text'},String(m.text??m.content??'')),m.outcomeUnknown===true?node('small',{class:'error'},'结果未知，未自动重试 · Outcome unknown'):m.streaming?node('small',{class:'muted'},'Receiving…'):['failed','cancelled','interrupted','refused','incomplete'].includes(m.status)?node('small',{class:'error'},`Run ${m.status} · no automatic retry`):null)));
  if(!s.messages.length){const welcome=node('div',{class:'welcome'},node('h3',{},s.loading?'Reading conversation…':!s.projects.length?'Chat first, create a project when you are ready':'What would you like to work on?'),node('p',{},s.loading?'Waiting for the host.':!s.projects.length?'Start chatting in a scratch workspace. You can name and keep it as a real project anytime.':'This chat has no dependency on an installed business application.'));if(!s.loading&&!s.projects.length)welcome.append(button('Start chatting',()=>void action(()=>controller.startScratchConversation()),{class:'primary','data-welcome-scratch':true}));else if(!s.loading&&s.projectId!==null&&!s.conversations.length)welcome.append(button('New conversation',()=>void action(()=>controller.createConversation()),{class:'primary','data-welcome-conversation':true}));messages.append(welcome);}
  if(nearEnd)messages.scrollTop=messages.scrollHeight;
  const activePlan=s.plans.get(entityId(s.conversationId));
  if(activePlan?.steps?.length){planProgress.hidden=false;planProgress.replaceChildren(node('h3',{},'Plan progress'),node('ol',{class:'plan-steps'},...activePlan.steps.map(step=>node('li',{'data-step-id':step.id,'data-status':step.status||'pending'},node('span',{class:'plan-step-title'},step.title||step.id),node('small',{class:'muted'},step.status||'pending')))));}
  else{planProgress.hidden=true;planProgress.replaceChildren();}
  const currentApprovals=s.approvals.filter(p=>entityId(p.conversationId)===entityId(s.conversationId));approvals.replaceChildren(...currentApprovals.map(p=>{const body=node('article',{class:'approval','data-permission-id':p.id},node('h3',{},p.title||'Runtime permission requested'),node('p',{},p.description||'Review the exact request before choosing an option.'),node('small',{class:'muted'},`Run ${p.runId||'unknown'}`));for(const option of p.options||[])body.append(button(option.label||option.name||String(option.id),()=>void action(()=>controller.approve(p.id,option.id??option.optionId)),{'data-permission-option':option.id??option.optionId}));if(!p.options?.length)body.append(node('p',{class:'error'},'No valid approval options were supplied. Nothing has been authorized.'));return body;}));
  const currentInsights=s.insightConfirms.filter(p=>entityId(p.projectId)===entityId(s.projectId));insightConfirms.replaceChildren(...currentInsights.map(p=>{const pending=controller.pendingInsightConfirms.has(entityId(p.proposalId));const body=node('article',{class:'approval insight-confirm','data-proposal-id':p.proposalId},node('h3',{},'Confirm insight write'),node('p',{},p.diffSummary||'Review the proposed insight patch before committing.'),node('small',{class:'muted'},`Record ${p.inventoryRecordId||'?'} · proposal ${p.proposalId}`));body.append(button('Confirm',()=>void action(()=>controller.confirmInsight(p.proposalId)),{class:'primary',disabled:pending,'data-insight-confirm':true}),button('Cancel',()=>controller.cancelInsight(p.proposalId),{disabled:pending,'data-insight-cancel':true}));return body;}));
  const run=s.runs.get(entityId(s.conversationId));cancel.hidden=!run;cancel.disabled=run?.state==='cancelling';
  const beat=s.heartbeats.get(entityId(s.conversationId));
  if(run){
    runKeepalive.hidden=false;
    if(beat&&Number.isFinite(beat.elapsedMs)){
      const secs=Math.max(0,Math.round(beat.elapsedMs/1000));
      runKeepalive.textContent=beat.stepTitle?`仍在运行 · ${secs}s · ${beat.stepTitle}`:`仍在运行 · ${secs}s`;
    }else runKeepalive.textContent='仍在运行…';
  }else{runKeepalive.hidden=true;runKeepalive.textContent='';}
  draft.value=controller.draft();updateComposer();
  if(appUi.open)renderApps();
  if(focused&&$(focused)){const element=$(focused);element.focus({preventScroll:true});if(typeof start==='number'&&element.setSelectionRange)element.setSelectionRange(start,end);}
}
function permissions(app){const values=app.permissions||app.manifest?.permissions||[];return values.map(p=>typeof p==='string'?{id:p,description:p}:p).filter(p=>typeof p.id==='string');}
function renderApps(){
  applicationRefresh.sync();
  const s=controller.state;$('app-error').textContent=appUi.error;$('app-error').hidden=!appUi.error;$('install-app').disabled=appUi.pending.has('install');
  const cards=s.apps.map(app=>{
    const id=String(app.id),pending=appUi.pending.has(id),busy=Boolean(app.busy),card=node('article',{class:'application','data-app-id':id});
    card.append(node('header',{},node('div',{},node('h3',{},app.name||id),node('small',{class:'muted'},`${id} · ${app.version||'version unknown'}`)),node('span',{class:'status'},app.enabled?'Enabled':'Disabled')));
    if(app.manifest?.schemaVersion===2)card.append(node('p',{class:'muted small'},'Application pages have separate operator actions. Permissions for application-wide configuration or rules affect the whole application; these actions are not model tools.'));
    if(app.busy)card.append(node('p',{class:'notice'},'Application busy. Uninstall is refused until accepted work settles. Disable immediately stops new admission; accepted background work continues.'));
    const actions=node('div',{class:'actions'},button(app.enabled?'Disable':'Enable',()=>void appAction(id,async()=>{const result=await api.post(`/api/apps/${segment(id)}/state`,{enabled:!app.enabled});if(result?.pairedSkillsPreview)appUi.pairedPreviews.set(id,result.pairedSkillsPreview);}),{disabled:pending,'data-app-action':'toggle'}),button('Uninstall',()=>{appUi.uninstall=id;renderApps();},{disabled:pending||busy,'data-app-action':'uninstall'}));
    for(const page of app.pages||[])actions.append(button(page.title||page.id,()=>{try{openPage(app,page);appDialog.close();}catch(error){appUi.error=error.message;renderApps();}},{disabled:pending||!app.enabled,'data-app-page':page.id}));card.append(actions);
    if(appUi.uninstall===id)card.append(node('div',{class:'uninstall-confirm',role:'alert'},node('p',{},'Uninstall this application? Application data will be preserved. This does not delete projects or conversations.'),button('Cancel',()=>{appUi.uninstall=null;renderApps();}),button('Uninstall · keep data',()=>void appAction(id,async()=>{await api.delete(`/api/apps/${segment(id)}`);appUi.uninstall=null;if(appUi.page?.appId===id)closePage();}),{disabled:pending||busy,class:'danger','data-app-action':'confirm-uninstall'})));
    if(!appUi.upgrades.has(id))appUi.upgrades.set(id,new UpgradeDraft());
    const upgrade=appUi.upgrades.get(id),upgradeState=upgrade.state,inputId=`upgrade-path-${id}`;
    const upgradeInput=node('input',{id:inputId,required:true,autocomplete:'off',disabled:pending,value:upgradeState.path,onInput:event=>{upgrade.setPath(event.target.value);render();}});
    const upgradeConsent=node('input',{type:'checkbox',disabled:pending||!upgradeSnapshot(app,upgradeState.path),onChange:event=>{upgrade.confirm(app,event.target.checked);renderApps();},'data-upgrade-confirm':id});upgradeConsent.checked=upgrade.ready(app);
    const upgradeForm=node('form',{class:'upgrade-form',onSubmit:event=>{event.preventDefault();if(!upgrade.ready(app))return;void appAction(id,async()=>{const result=await upgrade.run(api,app);if(result.items)controller.state.apps=result.items;if(!result.ok)throw Error(upgrade.state.error||'Upgrade state could not be verified.');if(appUi.page?.appId===id)closePage();});}},node('p',{class:'muted small'},`Observed ${app.version} · revision ${app.revision??'unknown'}. Upgrade only after explicitly disabling this application and waiting for accepted work to settle.`),node('label',{for:inputId},'New local package directory'),upgradeInput,node('label',{class:'skill-choice'},upgradeConsent,node('span',{},'Confirm upgrade from this directory at the displayed revision. The application stays disabled and permissions require review.')),node('button',{type:'submit',class:'primary',disabled:pending||!upgrade.ready(app),'data-upgrade-submit':id},'Upgrade application'),upgradeState.error?node('p',{class:'error',role:'alert'},upgradeState.error):null,upgradeState.notice?node('p',{class:'notice',role:'status'},upgradeState.notice):null);
    card.append(node('details',{class:'application-upgrade',open:upgradeState.open,onToggle:event=>{upgradeState.open=event.target.open;}},node('summary',{},'Upgrade from local package'),upgradeForm));
    const paired=appUi.pairedPreviews.get(id)||app.pairedSkillsPreview||null;
    const pairedSkills=paired?.skills||pairedSkillsSummary(app);
    if(pairedSkills.length){
      if(paired&&!appUi.pairedPreviews.has(id))appUi.pairedPreviews.set(id,paired);
      const pairedSection=node('section',{class:'skill-preview','data-paired-skills':id},node('h4',{},'Paired Skills'),node('p',{class:'muted small'},paired?.note||'Reference guidance declared by this application. Importing grants no tools or execution. Uninstall keeps Skills.'));
      for(const skill of pairedSkills){
        pairedSection.append(node('div',{class:'skill-entry','data-paired-skill':skill.name},node('span',{},skill.name,node('small',{class:'muted'},` · ${skill.source} · ${skill.status||'declared'}${skill.inSkillsReadScope?' · in skills_read':''}`)),node('div',{class:'actions'})));
        if(skill.excerpt)pairedSection.append(node('pre',{class:'resource-text'},skill.excerpt));
      }
      const importable=pairedSkills.filter(skill=>skill.importable||skill.status==='package_reference').map(skill=>skill.name);
      const includeable=pairedSkills.some(skill=>skill.inSkillsReadScope||skill.importable||skill.status==='in_library'||skill.status==='package_reference'||skill.status==='bundled_reference');
      pairedSection.append(node('div',{class:'actions'},
        button('Refresh preview',()=>void appAction(`${id}:paired-preview`,async()=>{appUi.pairedPreviews.set(id,await api.request(`/api/apps/${segment(id)}/paired-skills`));}),{disabled:pending,'data-paired-action':'preview'}),
        button('Import package Skills',()=>void appAction(`${id}:paired-import`,async()=>{const body=pairedSkillsIncludeBody({names:importable.length?importable:undefined,includeInSkillsRead:false});const result=await api.post(`/api/apps/${segment(id)}/paired-skills/import`,body);appUi.pairedPreviews.set(id,await api.request(`/api/apps/${segment(id)}/paired-skills`));appUi.pairedNotice=result.note||'Paired Skills imported. Reference only; no execution granted.';skillsPanel.invalidate?.();}),{disabled:pending||!importable.length,'data-paired-action':'import'}),
        button('Include in skills_read',()=>void appAction(`${id}:paired-include`,async()=>{const body=pairedSkillsIncludeBody({conversationId:controller.state.conversationId,includeInSkillsRead:true});const result=await api.post(`/api/apps/${segment(id)}/paired-skills/import`,body);appUi.pairedPreviews.set(id,await api.request(`/api/apps/${segment(id)}/paired-skills`));appUi.pairedNotice=result.note||'Paired Skills included in skills_read (reference only).';skillsPanel.invalidate?.();}),{disabled:pending||!includeable||controller.state.conversationId===null,'data-paired-action':'include'})
      ));
      if(appUi.pairedNotice)pairedSection.append(node('p',{class:'notice',role:'status'},appUi.pairedNotice));
      card.append(pairedSection);
    }

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
function clearNavigationTransition(page){if(page?.navigationTransition){clearTimeout(page.navigationTransition.timer);delete page.navigationTransition;}}
function closePage(){clearNavigationTransition(appUi.page);appUi.page?.navigationReview?.cancel();appUi.page=null;appUi.epoch++;appPage.replaceChildren();appPage.hidden=true;chat.hidden=false;}
function pageIsCurrent(current){const app=controller.state.apps.find(item=>item.id===current.appId),grant=app?.grants?.find(item=>entityId(item.projectId)===entityId(current.projectId));return appUi.page===current&&!current.authorityInvalidated&&entityId(controller.state.projectId)===entityId(current.projectId)&&app?.enabled&&app.version===current.version&&app.revision===current.appRevision&&(grant?.revision??0)===current.grantRevision;}
function reviewPageNavigation(current,prepared){
  if(current.navigationReview)throw Object.assign(Error('host_navigation_busy'),{code:'host_navigation_busy'});
  return new Promise(resolve=>{
    let settled=false,timer;
    const finish=accepted=>{if(settled)return;settled=true;clearTimeout(timer);dialog.close();dialog.remove();delete current.navigationReview;resolve(accepted);};
    const dialog=node('dialog',{'data-page-navigation':prepared.kind,onCancel:event=>{event.preventDefault();finish(false);}},node('h2',{},prepared.kind==='external'?'Review external link':'Review application handoff'),node('p',{},`From ${current.appId} · current project ${current.projectId}`),prepared.kind==='external'?node('p',{},prepared.url):node('div',{},node('p',{},`To ${prepared.targetAppId} · ${prepared.targetPageId}. The receiving application must review this data before creating a draft.`),node('pre',{class:'navigation-preview'},JSON.stringify(prepared.preview,null,2))),button('Cancel',()=>finish(false),{'data-navigation-cancel':true}),button(prepared.kind==='external'?'Offer link':'Send for review',()=>finish(true),{'data-navigation-confirm':true}));
    current.navigationReview={cancel:()=>finish(false)};document.body.append(dialog);dialog.showModal();timer=setTimeout(()=>finish(false),Math.max(0,prepared.expiresAt-Date.now()));
  });
}
function openPage(app,page,{deliveryToken}={}){
  clearNavigationTransition(appUi.page);appUi.page?.navigationReview?.cancel();
  const url=appPageUrl(page.url,app.id,location.origin),nonce=crypto.randomUUID(),epoch=++appUi.epoch;
  const projectId=controller.state.projectId,grant=(app.grants||[]).find(item=>entityId(item.projectId)===entityId(projectId));
  const protocol=app.manifest?.schemaVersion===2?NATIVE_BRIDGE_PROTOCOL:BRIDGE_PROTOCOL;
  if(protocol===NATIVE_BRIDGE_PROTOCOL&&projectId===null)throw Error('Choose a project before opening this application.');
  const frame=node('iframe',{src:url,sandbox:'allow-scripts',title:`${app.name||app.id}: ${page.title||page.id}`,referrerpolicy:'no-referrer','data-app-frame':app.id});
  const close=button('Close page · return to Chat',closePage);appUi.page={appId:app.id,pageId:page.id,version:app.version,appRevision:app.revision,grantRevision:grant?.revision??0,projectId,nonce,frame,epoch,protocol,lastRequestId:0,pending:0};
  frame.addEventListener('load',async()=>{const current=appUi.page;if(current?.epoch!==epoch)return;try{let handoff;if(deliveryToken){const token=deliveryToken;deliveryToken=undefined;handoff=await api.post(`/api/apps/${segment(app.id)}/navigation/consume`,{token,scope:pageScope(current)});if(!pageIsCurrent(current))return;}if(appUi.page===current)frame.contentWindow.postMessage({protocol,type:'init',appId:app.id,projectId,projectName:controller.state.projects.find(item=>entityId(item.id)===entityId(projectId))?.name??'',appRevision:app.revision,grantRevision:grant?.revision??0,nonce,...(handoff?{handoff}:{})},'*');}catch(error){if(appUi.page===current){current.authorityInvalidated=true;appPage.prepend(node('p',{class:'error',role:'alert'},`Application handoff was not delivered: ${error.message}. Nothing was retried.`));}}});
  appPage.replaceChildren(node('header',{class:'chat-header'},node('h2',{},page.title||app.name||app.id),close),node('p',{class:'muted small'},'Application page · isolated frame. Closing this page does not disable the application.'),frame);chat.hidden=true;appPage.hidden=false;
}
window.addEventListener('message',async event=>{
  const current=appUi.page;if(!current)return;
  const message=bridgeMessage(event,{source:current.frame.contentWindow,appId:current.appId,nonce:current.nonce,protocol:current.protocol});if(!message)return;
  if(message.type==='close')closePage();else if(message.type==='navigation.received'){const delivery=current.navigationTransition;if(delivery?.requestId===message.requestId&&pageIsCurrent(current)){clearNavigationTransition(current);delivery.open();}}else if(message.type==='resize')current.frame.style.height=`${message.height}px`;else if(message.type==='notice'){controller.state.notice=message.text;render();}
  else if(['action.call','tool.call','artifact.download','navigation.external','navigation.handoff','navigation.capabilities'].includes(message.type)){
    if(message.requestId<=current.lastRequestId)return;current.lastRequestId=message.requestId;
    const response={protocol:current.protocol,type:'result',appId:current.appId,nonce:current.nonce,requestId:message.requestId};
    let transition;
    try{
      if(current.pending>=32)throw Object.assign(Error('host_app_busy'),{code:'host_app_busy'});
      const app=controller.state.apps.find(item=>item.id===current.appId),grant=app?.grants?.find(item=>entityId(item.projectId)===entityId(current.projectId));
      if(current.authorityInvalidated||!app?.enabled||app.version!==current.version||app.revision!==current.appRevision||(grant?.revision??0)!==current.grantRevision||entityId(controller.state.projectId)!==entityId(current.projectId))throw Object.assign(Error('host_app_page_stale'),{code:'host_app_page_stale'});
      const navigation=message.type.startsWith('navigation.'),capabilityRead=message.type==='navigation.capabilities';
      const spec=navigation?{permission:message.type==='navigation.external'?'ui.external.open':'ui.handoff'}:(message.type==='tool.call'?app.manifest.tools:app.manifest.actions)?.find(item=>item.name===message.name);
      if(!spec||!capabilityRead&&!grant?.permissions?.includes(spec.permission))throw Object.assign(Error('host_action_permission_denied'),{code:'host_action_permission_denied'});
      current.pending++;
      try{
        if(capabilityRead){response.result=await api.post(`/api/apps/${segment(current.appId)}/navigation/capabilities`,{scope:pageScope(current),input:message.arguments});}
        else if(navigation){
          if(current.navigationTransition)throw Object.assign(Error('host_navigation_busy'),{code:'host_navigation_busy'});
          const result=await reviewNavigation({api,page:current,type:message.type,input:message.arguments,isCurrent:()=>pageIsCurrent(current),review:prepared=>reviewPageNavigation(current,prepared)});
          if(result.cancelled)response.result={cancelled:true};
          else if(result.kind==='external'){current.externalLink?.remove();const link=node('a',{href:result.url,target:'_blank',rel:'noopener noreferrer','data-external-link':true,class:'notice'},`Open external link: ${result.url}`);current.externalLink=link;appPage.prepend(link);response.result={offered:true};}
          else{
            await refreshApps();if(!pageIsCurrent(current))throw Object.assign(Error('host_app_page_stale'),{code:'host_app_page_stale'});
            const target=controller.state.apps.find(item=>item.id===result.targetAppId),targetPage=target?.pages?.find(item=>item.id===result.targetPageId);
            if(!target?.enabled||!targetPage)throw Object.assign(Error('host_navigation_target_unavailable'),{code:'host_navigation_target_unavailable'});
            response.result={handoffAccepted:true,draftOnly:true};transition=()=>openPage(target,targetPage,{deliveryToken:result.deliveryToken});
          }
        }else if(message.type==='artifact.download'){
          if(spec.delivery!=='artifact-read'||Object.keys(message.arguments).sort().join(',')!=='artifactId,filename')throw Object.assign(Error('host_artifact_request_invalid'),{code:'host_artifact_request_invalid'});
          const ticket=await api.post(`/api/apps/${segment(current.appId)}/artifact-downloads`,{projectId:current.projectId,pageId:current.pageId,version:current.version,expectedAppRevision:current.appRevision,expectedGrantRevision:current.grantRevision,actionName:message.name,artifactId:message.arguments.artifactId,filename:message.arguments.filename});
          const latest=controller.state.apps.find(item=>item.id===current.appId),liveGrant=latest?.grants?.find(item=>entityId(item.projectId)===entityId(current.projectId));
          if(current.authorityInvalidated||appUi.page!==current||entityId(controller.state.projectId)!==entityId(current.projectId)||!latest?.enabled||latest.version!==current.version||latest.revision!==current.appRevision||liveGrant?.revision!==current.grantRevision)throw Object.assign(Error('host_app_page_stale'),{code:'host_app_page_stale'});
          const url=new URL(ticket.url,location.origin);if(url.origin!==location.origin||!/^\/downloads\/[a-f0-9]{64}$/u.test(url.pathname)||url.search||url.hash)throw Error('invalid download');
          current.downloadLink?.remove();const link=node('a',{href:url.href,download:ticket.filename,rel:'noopener noreferrer',class:'notice'},`Download ${ticket.filename} · ${ticket.totalBytes} bytes`);current.downloadLink=link;appPage.prepend(link);link.click();
          response.result={downloadStarted:true,totalBytes:ticket.totalBytes,sha256:ticket.sha256};
        }else response.result=await api.post(`/api/apps/${segment(current.appId)}/${message.type==='action.call'?'actions':'tools'}/${segment(message.name)}`,{projectId:current.projectId,arguments:message.arguments,operationId:message.operationId,pageId:current.pageId,version:current.version,expectedAppRevision:current.appRevision,expectedGrantRevision:current.grantRevision,applicationScopeAccepted:spec.scope==='application'});
      }finally{current.pending--;}
    }catch(error){response.error={code:typeof error.code==='string'&&/^[A-Za-z0-9_]{1,96}$/u.test(error.code)?error.code:'app_page_action_failed'};}
    if(appUi.page===current&&entityId(controller.state.projectId)===entityId(current.projectId)){
      const app=controller.state.apps.find(item=>item.id===current.appId),grant=app?.grants?.find(item=>entityId(item.projectId)===entityId(current.projectId));
      if(current.authorityInvalidated||!app?.enabled||app.version!==current.version||app.revision!==current.appRevision||(grant?.revision??0)!==current.grantRevision){delete response.result;response.error={code:'host_app_page_stale'};}
      if(transition&&!response.error){const timer=setTimeout(()=>{if(current.navigationTransition?.requestId===message.requestId){clearNavigationTransition(current);if(appUi.page===current)appPage.prepend(node('p',{class:'notice',role:'status'},'The application did not acknowledge handoff delivery. Nothing was retried.'));}},30000);current.navigationTransition={requestId:message.requestId,open:transition,timer};}
      current.frame.contentWindow.postMessage(response,'*');
    }
  }
});
async function connectEvents(){
  let delay=1000;
  while(!stopped.signal.aborted){try{await api.events(event=>{if((event.type==='runtime.status'&&!entityId(event.conversationId))||event.type==='host.snapshot'){appUi.consents.clear();skillsPanel.invalidate();}const page=appUi.page;if(event.type==='applications.changed'&&event.appId){if(eventInvalidatesPage(event,page)){page.authorityInvalidated=true;clearNavigationTransition(page);page.navigationReview?.cancel();page.externalLink?.remove();page.downloadLink?.remove();if(!page.accessNotice){page.accessNotice=node('p',{class:'notice',role:'status'},'Application access changed. Close and reopen this page to use its current permissions.');appPage.prepend(page.accessNotice);}}void refreshApps().catch(error=>controller.fail(error));}if(event.type==='applications.changed'&&page?.protocol===NATIVE_BRIDGE_PROTOCOL&&(event.projectId===undefined||entityId(event.projectId)===entityId(page.projectId)))page.frame.contentWindow.postMessage({protocol:page.protocol,type:'navigation.changed',appId:page.appId,nonce:page.nonce},'*');if(event.type==='application.activity'&&page?.protocol===NATIVE_BRIDGE_PROTOCOL&&page.appId===event.appId)page.frame.contentWindow.postMessage({protocol:page.protocol,type:'activity',appId:page.appId,nonce:page.nonce},'*');controller.event(event);},{signal:stopped.signal,onState:value=>{controller.state.connection=value;render();}});if(stopped.signal.aborted)break;controller.state.connection='disconnected';render();}catch(error){if(stopped.signal.aborted)break;controller.state.connection='disconnected';controller.state.error=`Event connection unavailable: ${error.message}. Messages are not resent automatically.`;render();}
    await new Promise(resolve=>{reconnectTimer=setTimeout(resolve,delay);stopped.signal.addEventListener('abort',resolve,{once:true});});delay=Math.min(delay*2,10000);
  }
}
window.addEventListener('pagehide',()=>{stopped.abort();clearTimeout(reconnectTimer);controller.dispose();},{once:true});
render();void controller.init();void connectEvents();
