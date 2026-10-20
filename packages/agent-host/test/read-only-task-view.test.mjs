import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp,rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { readOnlyAnswerPresentation } from '../src/tasks.mjs';
import { AgentHost } from '../src/service.mjs';
import { serveHost } from '../src/server.mjs';

const user={id:'user',role:'user',content:'核对当前项目扫描结果。只读取现有数据，不启动。'};
const assistant={id:'answer',role:'assistant',content:'根据保存结果，已完成本轮核对。',status:'completed',stopReason:'end_turn',runId:'read-run',outcomeUnknown:false};
const saved=()=>({status:'awaiting_input',outcomeUnknown:false,steps:[],children:[],acceptance:null,
  runs:[{id:'read-run',userMessageId:'user',assistantMessageId:'answer',status:'completed',outcomeUnknown:false}],
  evidence:[{runId:'read-run',readOnly:true,status:'completed',outcomeUnknown:false}]});

test('read answer presentation preserves historical acceptance, unknown effects and execution states',()=>{
 const task=saved();task.outcomeUnknown=true;task.steps=[{id:'old',status:'interrupted',verified:false}];task.acceptance={ok:false,runId:'older'};
 const before=structuredClone(task),view=readOnlyAnswerPresentation(task,[user,assistant]);
 assert.equal(view.kind,'read_only_answer');assert.equal(view.historicalWorkPending,true);assert.deepEqual(task,before);
});

test('failed, partial, empty, unmatched and write-bearing turns never claim a read answer',()=>{
 const variants=[
  {answer:{...assistant,status:'failed'}},{answer:{...assistant,status:'incomplete',stopReason:'max_turn_requests'}},
  {answer:{...assistant,content:''}},{answer:{...assistant,outcomeUnknown:true}},
  {answer:{...assistant,runId:'foreign'}},{question:{...user,content:'启动扫描任务'}},
  {evidence:[{runId:'read-run',readOnly:false,status:'completed'}]},
  {evidence:[{runId:'read-run',readOnly:true,status:'running'}]}, {evidence:[]},
 ];
 for(const value of variants){const task=saved();if(value.evidence)task.evidence=value.evidence;assert.equal(readOnlyAnswerPresentation(task,[value.question??user,value.answer??assistant]),null);}
 assert.equal(readOnlyAnswerPresentation(saved(),[user,assistant,{...user,id:'next-user'}]),null);
});

test('HTTP task view is derived from persisted messages without saving or changing the original task',async t=>{
 const dataDir=await mkdtemp(join(tmpdir(),'ovi-read-task-view-'));t.after(()=>rm(dataDir,{recursive:true,force:true}));const host=new AgentHost({dataDir,runtimeSelection:'none'});t.after(()=>host.close());
 const project=host.store.createProject('one'),conversation=host.store.createConversation(project.id,'read'),task=host.store.createTask(conversation.id,user.content);
 const question=host.store.addMessage(conversation.id,'user',user.content),answer=host.store.addMessage(conversation.id,'assistant',assistant.content,'completed','read-run');host.store.finishMessage(answer.id,assistant.content,'completed',{stopReason:'end_turn',outcomeUnknown:false});
 const data={...task,...saved(),runs:[{id:'read-run',status:'completed',userMessageId:question.id,assistantMessageId:answer.id}]};host.store.saveTask(data);
 const before=host.store.task(conversation.id),changes=host.store.db.prepare('SELECT total_changes() AS n').get().n;
 const web=await serveHost(host);t.after(()=>web.close());
 const response=await fetch(`${web.origin}/api/conversations/${conversation.id}/task?projectId=${project.id}`,{headers:{'X-Oviraptor-Session':web.token}});
 assert.equal(response.status,200);assert.equal((await response.json()).task.readOnlyAnswer.kind,'read_only_answer');
 assert.deepEqual(host.store.task(conversation.id),before);assert.equal(host.store.db.prepare('SELECT total_changes() AS n').get().n,changes);
 const other=host.store.createProject('other');assert.throws(()=>host.taskView(conversation.id,{projectId:other.id}));
});
