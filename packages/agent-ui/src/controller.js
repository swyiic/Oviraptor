import {entityId,segment} from './api.js';
export const MESSAGE_TERMINAL_STATUSES=['completed','cancelled','failed','interrupted','refused','incomplete'];
const same=(a,b)=>entityId(a)===entityId(b);
export class ChatController {
  constructor(api,changed=()=>{}){this.api=api;this.changed=changed;this.epoch=0;this.messageRevision=0;this.disposed=false;this.pendingWrites=new Set();this.pendingApprovals=new Set();this.pendingCancels=new Set();this.finishedRuns=new Set();this.drafts=new Map();this.state={projects:[],projectId:null,conversations:[],conversationId:null,messages:[],apps:[],runtime:{state:'unknown'},connection:'connecting',approvals:[],runs:new Map(),plans:new Map(),heartbeats:new Map(),loading:false,error:'',notice:''};}
  update(){if(!this.disposed)this.changed(this.state);}
  fail(error){this.state.error=error?.message||String(error);this.update();}
  draft(){return this.drafts.get(entityId(this.state.conversationId))||'';}
  setDraft(text){if(this.state.conversationId!==null)this.drafts.set(entityId(this.state.conversationId),text);}
  async init(){
    const [projects,status,apps]=await Promise.allSettled([this.api.list('/api/projects'),this.api.request('/api/status'),this.api.list('/api/apps')]);
    if(this.disposed)return;
    if(projects.status==='fulfilled')this.state.projects=projects.value;else this.fail(projects.reason);
    if(status.status==='fulfilled')this.applyStatus(status.value);else this.fail(status.reason);
    if(apps.status==='fulfilled')this.state.apps=apps.value;else this.fail(apps.reason);
    if(this.state.projects.length)await this.selectProject(this.state.projects[0].id);else this.update();
  }
  applyStatus(value){if(value?.runtime)this.state.runtime=value.runtime;if(Array.isArray(value?.activeRuns))this.state.runs=new Map(value.activeRuns.map(run=>[entityId(run.conversationId),run]));if(Array.isArray(value?.permissions))this.state.approvals=value.permissions;}
  async selectProject(id){
    const epoch=++this.epoch;this.state.projectId=id;this.state.conversationId=null;this.state.conversations=[];this.state.messages=[];this.state.loading=true;this.state.error='';this.state.notice='';this.update();
    try{const rows=await this.api.list(`/api/conversations?projectId=${segment(id)}`);if(this.disposed||epoch!==this.epoch)return;this.state.conversations=rows;if(rows.length)await this.selectConversation(rows[0].id);}
    catch(error){if(epoch===this.epoch&&!this.disposed)this.fail(error);}finally{if(epoch===this.epoch){this.state.loading=false;this.update();}}
  }
  async selectConversation(id){
    const epoch=++this.epoch;this.state.conversationId=id;this.state.messages=[];this.state.error='';this.state.notice='';this.state.loading=true;this.update();
    try{await this.reloadMessages(id,epoch);}catch(error){if(epoch===this.epoch&&!this.disposed)this.fail(error);}finally{if(epoch===this.epoch){this.state.loading=false;this.update();}}
  }
  async reloadMessages(id=this.state.conversationId,epoch=this.epoch){
    const revision=this.messageRevision;const messages=await this.api.list(`/api/conversations/${segment(id)}/messages`);
    if(this.disposed||epoch!==this.epoch||!same(this.state.conversationId,id))return;
    if(revision===this.messageRevision&&!this.state.messages.some(m=>m.streaming))this.state.messages=messages;
    else {const byId=new Map(messages.map(m=>[entityId(m.id),m]));for(const m of this.state.messages){const stored=byId.get(entityId(m.id));if(!stored||(m.streaming&&!MESSAGE_TERMINAL_STATUSES.includes(stored.status)))byId.set(entityId(m.id),m);}this.state.messages=[...byId.values()];}
    this.update();
  }
  async createProject(name,{kind}={}){const epoch=this.epoch;const body={name};if(kind==='scratch')body.kind='scratch';const row=await this.api.post('/api/projects',body);const projects=await this.api.list('/api/projects');if(this.disposed)return;this.state.projects=projects;if(epoch===this.epoch)await this.selectProject(row.id);else this.update();return row;}
  async createConversation(){if(this.state.projectId===null)return;const projectId=this.state.projectId,epoch=this.epoch;const row=await this.api.post('/api/conversations',{projectId,title:'New conversation'});if(this.disposed||epoch!==this.epoch||!same(projectId,this.state.projectId))return;const conversations=await this.api.list(`/api/conversations?projectId=${segment(projectId)}`);if(this.disposed||epoch!==this.epoch||!same(projectId,this.state.projectId))return;this.state.conversations=conversations;await this.selectConversation(row.id);}
  /** Empty-state / chat-first: one host call creates Scratch project + first conversation. */
  async startScratchConversation(){const epoch=this.epoch;const row=await this.api.post('/api/conversations',{scratch:true,title:'New conversation'});if(this.disposed)return null;const projects=await this.api.list('/api/projects');if(this.disposed)return null;this.state.projects=projects;if(epoch!==this.epoch){this.update();return row;}await this.selectProject(row.projectId);if(this.disposed||!same(this.state.projectId,row.projectId))return row;if(!same(this.state.conversationId,row.id))await this.selectConversation(row.id);return row;}
  /** Promote the selected scratch workspace to a named durable project. */
  async promoteCurrentProject(name){const projectId=this.state.projectId;if(projectId===null)return null;const epoch=this.epoch;const row=await this.api.post(`/api/projects/${segment(projectId)}/promote`,{name});const projects=await this.api.list('/api/projects');if(this.disposed)return null;this.state.projects=projects;if(epoch===this.epoch&&same(projectId,this.state.projectId))this.update();else this.update();return row;}
  currentProject(){return this.state.projects.find(p=>same(p.id,this.state.projectId))||null;}
  async send(){
    const id=this.state.conversationId,projectId=this.state.projectId,text=this.draft().trim(),key=entityId(id);
    if(id===null||!text||this.pendingWrites.has(key)||this.state.runs.has(key))return false;
    if(new TextEncoder().encode(text).length>16384){this.fail(Error('A message can contain at most 16,384 UTF-8 bytes.'));return false;}
    this.pendingWrites.add(key);this.state.error='';this.state.notice='';this.update();
    try{const result=await this.api.post(`/api/conversations/${segment(id)}/messages`,{text});
      if(result?.run&&!this.finishedRuns.has(entityId(result.run.id)))this.state.runs.set(key,result.run);
      if(this.drafts.get(key)?.trim()===text)this.drafts.set(key,'');
      if(same(id,this.state.conversationId)&&same(projectId,this.state.projectId))await this.reloadMessages(id);
      return true;
    }catch(error){if(same(id,this.state.conversationId)&&same(projectId,this.state.projectId))this.fail(error);return false;}finally{this.pendingWrites.delete(key);this.update();}
  }
  async cancel(){const id=this.state.conversationId,key=entityId(id),requestedRun=this.state.runs.get(key);if(id===null||!requestedRun||this.pendingCancels.has(key))return;this.pendingCancels.add(key);try{await this.api.post('/api/runtime/cancel',{conversationId:id});const run=this.state.runs.get(key);if(run&&same(run.id??run.runId,requestedRun.id??requestedRun.runId)){this.state.runs.set(key,{...run,state:'cancelling'});if(same(id,this.state.conversationId))this.state.notice='Cancellation requested. Waiting for the runtime receipt.';}this.update();}finally{this.pendingCancels.delete(key);}}
  async approve(id,optionId){const key=entityId(id),item=this.state.approvals.find(x=>same(x.id,id));if(!item||!same(item.conversationId,this.state.conversationId)||this.pendingApprovals.has(key)||!(item.options||[]).some(x=>same(x.id??x.optionId,optionId)))return;this.pendingApprovals.add(key);try{await this.api.post(`/api/runtime/permissions/${segment(id)}`,{optionId});this.state.approvals=this.state.approvals.filter(x=>!same(x.id,id));this.update();}finally{this.pendingApprovals.delete(key);}}
  event(event){
    if(this.disposed||!event||typeof event.type!=='string')return;
    if(event.type==='host.snapshot'){this.applyStatus(event);this.update();return;}
    if(event.type==='application.activity'&&typeof event.busy==='boolean'){
      const app=this.state.apps.find(item=>item.id===event.appId);if(app)app.busy=event.busy;
    }
    if(event.type==='conversation.updated'&&event.conversationId&&typeof event.title==='string'){
      const row=this.state.conversations.find(item=>same(item.id,event.conversationId));
      if(row)row.title=event.title;
    }
    if((event.type==='project.updated'||event.type==='project.created')&&event.projectId!=null){
      const row=this.state.projects.find(item=>same(item.id,event.projectId));
      if(row){if(typeof event.name==='string')row.name=event.name;if(typeof event.kind==='string')row.kind=event.kind;}
    }
    const id=entityId(event.conversationId),active=same(event.conversationId,this.state.conversationId);
    if(event.type==='runtime.heartbeat'&&id){
      const current=this.state.runs.get(id);
      if(current&&(!event.runId||same(current.id??current.runId,event.runId))){
        this.state.heartbeats.set(id,{runId:event.runId,elapsedMs:event.elapsedMs,lastEventAt:event.lastEventAt,stepTitle:event.stepTitle,stepStatus:event.stepStatus});
      }
    }
    if(event.type==='runtime.status'){
      const status=event.state||event.status;if(!id){if(status)this.state.runtime={...this.state.runtime,state:status,message:event.message,...(typeof event.identity==='string'?{identity:event.identity}:{})};}
      else if(['running','starting','waiting_permission','cancelling'].includes(status))this.state.runs.set(id,{...event,id:event.runId||event.id});
      else if([...MESSAGE_TERMINAL_STATUSES,'idle','unconfigured'].includes(status)){const current=this.state.runs.get(id);if(!current||!event.runId||same(current.id??current.runId,event.runId)){this.state.runs.delete(id);this.state.heartbeats.delete(id);this.state.approvals=this.state.approvals.filter(p=>!same(p.conversationId,event.conversationId)||Boolean(event.runId&&!same(p.runId,event.runId)));}}
    }
    if(event.type==='permission.request'&&event.id&&!this.state.approvals.some(x=>same(x.id,event.id)))this.state.approvals.push(event);
    if(event.type==='runtime.error'&&active)this.state.error=event.message||event.code||'The runtime failed. The request has not been resent.';
    if(event.type==='runtime.plan'&&id){
      const steps=Array.isArray(event.steps)?event.steps.map((step,index)=>({id:String(step.id??`step-${index}`),title:String(step.title??step.id??`Step ${index+1}`),status:step.status||'pending',index:Number.isInteger(step.index)?step.index:index})): [];
      this.state.plans.set(id,{runId:event.runId,steps});
    }
    if(event.type==='runtime.step'&&id){
      const plan=this.state.plans.get(id)||{runId:event.runId,steps:[]};
      const stepId=String(event.id??event.stepId??`step-${plan.steps.length}`);
      let step=plan.steps.find(item=>same(item.id,stepId));
      if(!step){step={id:stepId,title:String(event.title??stepId),status:event.status||'in_progress',index:plan.steps.length};plan.steps.push(step);}
      else{step.title=String(event.title??step.title);step.status=event.status||step.status;}
      plan.runId=event.runId||plan.runId;this.state.plans.set(id,plan);
    }
    if(event.type==='message.delta'||event.type==='message.completed'){
      if(event.type==='message.completed'){
        this.finishedRuns.add(entityId(event.runId));if(this.finishedRuns.size>1000)this.finishedRuns.delete(this.finishedRuns.values().next().value);
        const current=this.state.runs.get(id);if(!current||!event.runId||same(current.id??current.runId,event.runId)){this.state.runs.delete(id);this.state.plans.delete(id);this.state.heartbeats.delete(id);}
        this.state.approvals=this.state.approvals.filter(p=>!same(p.conversationId,event.conversationId)||Boolean(event.runId&&!same(p.runId,event.runId)));
        if(active&&(this.pendingCancels.has(id)||current?.state==='cancelling'))this.state.notice=event.outcomeUnknown===true?'结果未知，未自动重试。Runtime result is unknown.':`Run ${MESSAGE_TERMINAL_STATUSES.includes(event.status)?event.status:'finished'}. No automatic retry.`;
      }
      if(!active){this.update();return;}this.messageRevision++;
      const message=event.message||event;
      const byRun=event.runId?this.state.messages.findIndex(x=>same(x.runId,event.runId)&&x.role==='assistant'):-1;
      const messageId=event.message?.id||event.messageId||(byRun>=0?this.state.messages[byRun].id:null)||message.id||`run-${event.runId}`;
      let index=this.state.messages.findIndex(x=>same(x.id,messageId));if(index<0)index=byRun;const old=index<0?null:this.state.messages[index];
      const text=event.type==='message.delta'?(old?.text||old?.content||'')+(event.delta??message.delta??event.text??''):(message.text??message.content??old?.text??'');
      const row={...old,...message,id:messageId,role:message.role||'assistant',text,streaming:event.type==='message.delta'};
      if(index<0)this.state.messages.push(row);else this.state.messages[index]=row;
    }
    this.update();
  }
  dispose(){this.disposed=true;this.epoch++;}
}
