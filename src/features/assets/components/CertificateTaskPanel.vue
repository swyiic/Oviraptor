<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { Play, Pause, RefreshCw, ShieldCheck, Clock3 } from '@lucide/vue';
const props=defineProps<{projectId?:number;archived?:boolean}>();
const emit=defineEmits<{changed:[]}>();
type Progress={total:number;captured:number;failed:number;skipped:number;pending:number;running:number;uniqueFingerprints:number;newFingerprints:number;attempts:number};
type Task={createdAt?:string;selection?:{roots?:string[];httpsAssets:number;outsideConfirmedScope:number;alreadyHaveFingerprint:number;queued:number};id:string;status:string;reason:string;workers:number;progress:Progress;items:{assetId:number;endpoint:string;status:string;reason:string;attempts:number;fingerprint:string}[]};
type View={roots:string[];tasks:Task[]};
const data=ref<View>(),busy=ref(false),reading=ref(false),error=ref(''),workers=ref(4),expanded=ref(false),taskId=ref('');
let generation=0,sequence=0,disposed=false,timer:ReturnType<typeof setTimeout>|undefined;
let readOwner:{generation:number;promise:Promise<void>}|undefined;
const task=computed(()=>data.value?.tasks.find(t=>t.id===taskId.value)??data.value?.tasks[0]);
const active=computed(()=>task.value&&['running','pausing'].includes(task.value.status));
const done=computed(()=>task.value?task.value.progress.captured+task.value.progress.failed+task.value.progress.skipped:0);
const percent=computed(()=>task.value?.progress.total?Math.min(100,Math.floor(done.value/task.value.progress.total*100)):0);
const taskRoots=computed(()=>task.value?.selection?.roots);
const scopeChanged=computed(()=>!!taskRoots.value&&JSON.stringify([...taskRoots.value].sort())!==JSON.stringify([...(data.value?.roots||[])].sort()));
const historical=computed(()=>!!task.value&&(!taskRoots.value?.length||scopeChanged.value||task.value.id!==data.value?.tasks[0]?.id));
const canStartCurrent=computed(()=>!task.value||task.value.status==='completed'||(!taskRoots.value?.length||scopeChanged.value)&&!active.value);
const hasSelectionCounts=computed(()=>{const selection=task.value?.selection;return !!selection&&(['httpsAssets','outsideConfirmedScope','alreadyHaveFingerprint','queued'] as const).every(key=>typeof selection[key]==='number');});
const labels:Record<string,string>={running:'正在补采',pausing:'正在保存并暂停',paused:'已暂停',interrupted:'应用退出，等待恢复',completed:'处理完成',failed:'执行中断，请检查原因',pending:'待处理',captured:'证书已保存',skipped:'已跳过'};
function schedule(g=generation){clearTimeout(timer);if(!disposed&&g===generation&&data.value?.tasks.some(t=>['running','pausing'].includes(t.status)))timer=setTimeout(()=>void load(g),1500);}
function applyView(value:View){
 const old=task.value;data.value=value;
 if(!value.tasks.some(t=>t.id===taskId.value))taskId.value=value.tasks[0]?.id??'';
 if(task.value&&!['completed','failed'].includes(task.value.status))workers.value=task.value.workers;
 if(old?.status==='running'&&task.value?.status!=='running')emit('changed');
}
function load(g=generation):Promise<void>{
 const projectId=props.projectId;if(!projectId||disposed||g!==generation)return Promise.resolve();
 if(readOwner?.generation===g)return readOwner.promise;
 reading.value=true;const seq=++sequence;
 const promise=(async()=>{
  try{const value=await invoke<View>('certificate_task_action',{projectId,action:'status',args:{}});if(disposed||g!==generation||seq!==sequence)return;applyView(value);error.value='';}
  catch(e){if(g===generation&&seq===sequence&&!disposed)error.value=String(e);}
  finally{if(g===generation&&seq===sequence&&!disposed){readOwner=undefined;reading.value=false;schedule(g);}}
 })();
 readOwner={generation:g,promise};return promise;
}
async function act(action:string,retryFailed=false){
 if(!props.projectId||busy.value||reading.value||props.archived)return;
 const projectId=props.projectId,g=generation;clearTimeout(timer);busy.value=true;error.value='';
 try{const result=await invoke<View>('certificate_task_action',{projectId,action,args:action==='start'?{workers:workers.value,timeoutSeconds:8}:{taskId:task.value?.id,retryFailed}});if(g===generation&&!disposed){taskId.value=result.tasks[0]?.id??taskId.value;applyView(result);await load(g);if(g===generation&&!disposed)emit('changed');}}
 catch(e){if(g===generation&&!disposed)error.value=String(e);}finally{if(g===generation&&!disposed){busy.value=false;schedule(g);}}
}
watch(taskId,()=>{expanded.value=false;});
watch(()=>props.projectId,()=>{generation++;sequence++;clearTimeout(timer);readOwner=undefined;data.value=undefined;taskId.value='';expanded.value=false;error.value='';busy.value=false;reading.value=false;void load();},{immediate:true});
onUnmounted(()=>{disposed=true;generation++;sequence++;clearTimeout(timer);});
</script>
<template>
 <section class="certificate-task-panel" aria-label="TLS 证书补采">
  <header class="certificate-task-heading">
   <div class="certificate-task-title"><span class="certificate-task-icon"><ShieldCheck :size="19"/></span><div><h3>读取缺失的 TLS 证书</h3><p>只读取证书，不获取网页；不消耗 FOFA / Hunter 积分。</p></div></div>
   <button type="button" v-refresh-feedback="{scope:projectId,busy:reading,error}" class="button ghost compact certificate-refresh" :disabled="busy||reading||!projectId" data-certificate-action="refresh" aria-label="刷新证书补采进度" @click="load()"><RefreshCw :size="14" :class="{spinning:reading}"/>{{reading?'正在读取…':'刷新进度'}}</button>
  </header>
  <div class="certificate-task-scope">
   <span class="scope-label">当前已确认范围</span><div v-if="data?.roots.length" class="scope-domains"><span v-for="root in data.roots" :key="root">{{root}}</span></div><span v-else class="scope-empty">{{reading?'正在读取范围…':'尚未设置，请先在「新建查询 → AI 发现资产」确认范围。'}}</span>
   <small>新补采按此范围选择 HTTPS 资产；范围外的 IP 入口不会自动加入。</small>
  </div>
  <div class="certificate-task-toolbar">
   <label v-if="data&&data.tasks.length>1" class="task-history"><span><Clock3 :size="13"/>任务记录</span><select v-model="taskId" aria-label="证书补采任务历史" :disabled="busy"><option v-for="item in data.tasks" :key="item.id" :value="item.id">{{item.createdAt||item.id.slice(0,8)}} · {{labels[item.status]||item.status}}</option></select></label>
   <div v-else-if="task" class="task-record-label"><span>{{historical?'历史任务':'最近任务'}}</span><strong>{{task.createdAt||task.id.slice(0,8)}}</strong></div>
   <label v-if="canStartCurrent" class="task-workers"><span>新任务同时读取</span><select v-model.number="workers" aria-label="TLS 补采并发数量" :disabled="busy||reading"><option :value="1">1 台</option><option :value="4">4 台</option><option :value="8">8 台</option><option :value="16">16 台</option></select></label>
   <div class="certificate-task-actions">
    <button type="button" v-if="canStartCurrent" class="button primary" :disabled="busy||reading||!projectId||archived||!data?.roots.length" data-certificate-action="start" @click="act('start')"><Play :size="14"/>{{busy?'正在接续…':task?'按当前范围检查并补采':'开始补采缺失证书'}}</button>
    <button type="button" v-if="task&&['running','pausing'].includes(task.status)" class="button ghost" :disabled="busy||reading||task.status==='pausing'||archived" data-certificate-action="pause" @click="act('pause')"><Pause :size="14"/>{{task.status==='pausing'?'正在暂停…':'暂停补采'}}</button>
    <button type="button" v-if="task&&['paused','interrupted','failed'].includes(task.status)" class="button" :class="{primary:!scopeChanged,ghost:scopeChanged}" :disabled="busy||reading||archived" data-certificate-action="resume" @click="act('resume')"><Play :size="14"/>{{busy?'正在接续…':historical?'继续此历史任务':'继续未完成项'}}</button>
    <button type="button" v-if="task&&!active&&task.progress.failed>0" class="button ghost" :disabled="busy||reading||archived" data-certificate-action="retry" @click="act('resume',true)">重试失败项 · {{task.progress.failed}}</button>
   </div>
  </div>
  <template v-if="task">
   <div v-if="scopeChanged||!taskRoots?.length" class="certificate-scope-change" role="status"><strong>{{taskRoots?.length?'当前查看的是旧范围的任务记录':'此历史任务没有记录创建时的范围'}}</strong><span>任务范围：{{taskRoots?.join('、')||'未记录'}}；当前范围：{{data?.roots.join('、')||'尚未设置'}}。</span><span>下方统计仅属于此任务，不代表当前范围的补采进度。继续旧任务时，范围外项目会跳过。</span></div>
   <div class="certificate-task-progress-heading"><div><strong>{{historical?'历史任务进度':'任务进度'}}</strong><span v-if="taskRoots?.length&&!scopeChanged">{{taskRoots.join('、')}}</span></div><span class="certificate-task-status" :class="{active:active,failed:task.status==='failed'}">{{labels[task.status]||task.status}}<template v-if="active"> · {{task.workers}} 台并发</template></span></div>
   <div class="certificate-task-metrics">
    <div><span>已处理资产</span><strong>{{done.toLocaleString()}}<small> / {{task.progress.total.toLocaleString()}}</small></strong></div>
    <div><span>证书已保存</span><strong>{{task.progress.captured.toLocaleString()}}</strong></div>
    <div><span>失败</span><strong :class="{warning:task.progress.failed>0}">{{task.progress.failed.toLocaleString()}}</strong></div>
    <div><span>跳过</span><strong>{{task.progress.skipped.toLocaleString()}}</strong></div>
    <div><span>唯一指纹</span><strong>{{task.progress.uniqueFingerprints.toLocaleString()}}</strong></div>
    <div><span>新增指纹</span><strong>{{task.progress.newFingerprints.toLocaleString()}}</strong></div>
   </div>
   <progress :value="done" :max="Math.max(1,task.progress.total)" :aria-label="`证书补采进度 ${percent}%`"/>
   <p class="certificate-task-note">{{task.reason}}{{task.reason&&/[。！？]$/.test(task.reason)?'':'。'}}<span v-if="!task.progress.total">{{historical?'此历史任务没有待补采项。':'本任务没有待补采项。'}}</span></p>
   <details class="certificate-task-details" :open="expanded" @toggle="expanded=($event.target as HTMLDetailsElement).open"><summary>任务详情与采集说明 · {{task.id.slice(0,8)}}</summary>
    <p v-if="hasSelectionCounts&&task.selection" class="certificate-task-selection">创建时 {{task.selection.httpsAssets.toLocaleString()}} 条 HTTPS 记录 · 范围外 {{task.selection.outsideConfirmedScope.toLocaleString()}} 条 · 已有指纹 {{task.selection.alreadyHaveFingerprint.toLocaleString()}} 条 · 本任务入队 {{task.selection.queued.toLocaleString()}} 条。</p>
    <div v-if="task.items.length" class="certificate-task-table"><table><thead><tr><th>资产</th><th>状态</th><th>说明</th></tr></thead><tbody><tr v-for="item in task.items" :key="item.assetId"><td>{{item.endpoint}}</td><td>{{labels[item.status]||item.status}}</td><td>{{item.reason||'等待执行'}}</td></tr></tbody></table></div>
    <p>同一张证书部署在多台资产上，只计一个唯一指纹。补采不会自动查询 Hunter，也不会解除隔离。最多展示 20 条；完整进度由任务记录保存，暂停会先保存当前握手结果。</p>
   </details>
  </template>
  <p v-else-if="!reading&&!error" class="certificate-task-empty">尚无补采任务。确认范围后，点击开始即可自动处理缺少指纹的 HTTPS 资产。</p>
  <p v-if="error" class="certificate-task-error" role="alert">{{error}}</p>
 </section>
</template>
<style scoped>
.certificate-task-panel{margin:0 0 18px;padding:20px;border:1px solid var(--line,#e5e5e5);border-radius:12px;background:var(--app-surface,#fff);font-size:13px;line-height:1.6;min-width:0;box-sizing:border-box}.certificate-task-heading{display:flex;justify-content:space-between;align-items:flex-start;gap:16px}.certificate-task-title{display:flex;align-items:flex-start;gap:10px;min-width:0}.certificate-task-icon{display:flex;align-items:center;justify-content:center;width:34px;height:34px;flex:0 0 34px;border-radius:8px;background:var(--app-bg,#f5f7fa);color:var(--muted,#6b7280)}.certificate-task-panel h3{font-size:15px;line-height:1.5;margin:0 0 3px}.certificate-task-panel p{margin:0;color:var(--muted,#777);font-size:12px}.certificate-task-panel .button{display:inline-flex;align-items:center;justify-content:center;gap:7px;white-space:nowrap;min-height:36px;padding:8px 12px;font:inherit;font-size:12px}.certificate-refresh{flex-shrink:0}.certificate-task-scope{display:grid;grid-template-columns:104px minmax(0,1fr);align-items:start;gap:6px 12px;padding:12px 14px;border:1px solid var(--line,#e5e5e5);border-radius:8px;background:var(--app-bg,#fafbfc);margin-top:16px;font-size:12px}.scope-label{font-size:11px;color:var(--muted);padding-top:3px}.scope-domains{display:flex;flex-wrap:wrap;gap:5px;min-width:0}.scope-domains>span{padding:2px 7px;border:1px solid var(--line);border-radius:5px;background:var(--app-surface,#fff);overflow-wrap:anywhere}.certificate-task-scope>small{grid-column:2;color:var(--muted);font-size:11px}.scope-empty{color:var(--muted)}.certificate-task-toolbar{display:flex;align-items:flex-end;gap:14px;margin:16px 0;flex-wrap:wrap}.certificate-task-toolbar label,.task-record-label{display:flex;flex-direction:column;gap:5px;min-width:0;font-size:11px;color:var(--muted)}.certificate-task-toolbar label>span{display:flex;align-items:center;gap:5px}.task-history{flex:1 1 260px;max-width:400px}.task-record-label{flex:1 1 180px}.task-record-label>strong{color:var(--text,#30343b);font-size:12px;line-height:36px;font-weight:500}.certificate-task-toolbar select{box-sizing:border-box;min-width:76px;max-width:100%;height:36px;padding:7px 28px 7px 10px;border:1px solid var(--line);border-radius:6px;background:var(--app-surface,#fff);font:inherit;font-size:12px;color:var(--text,#30343b)}.task-history select{width:100%}.certificate-task-actions{display:flex;gap:8px;flex-wrap:wrap;align-items:center;margin-left:auto}.certificate-scope-change{display:flex;flex-direction:column;gap:3px;padding:10px 12px;border:1px solid #ead9b8;border-radius:7px;background:#fffaf1;font-size:11px;color:#7b5d30;margin-bottom:14px}.certificate-scope-change strong{font-size:12px}.certificate-scope-change span{overflow-wrap:anywhere}.certificate-task-progress-heading{display:flex;align-items:flex-start;justify-content:space-between;gap:10px;margin:16px 0 10px}.certificate-task-progress-heading>div{display:flex;align-items:center;gap:9px;flex-wrap:wrap;min-width:0}.certificate-task-progress-heading strong{font-size:12px}.certificate-task-progress-heading>div>span{font-size:11px;color:var(--muted);overflow-wrap:anywhere}.certificate-task-status{padding:2px 7px;border:1px solid var(--line);border-radius:5px;color:var(--muted);font-size:11px;flex-shrink:0}.certificate-task-status.active{background:#edf8f2;color:#236e50;border-color:#cce6d8}.certificate-task-status.failed{color:#a0473c;border-color:#eed5d0;background:#fff7f6}.certificate-task-metrics{display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:8px}.certificate-task-metrics>div{padding:10px 12px;border:1px solid var(--line);border-radius:7px;min-width:0;background:var(--app-surface,#fff)}.certificate-task-metrics>div>span{display:block;font-size:11px;color:var(--muted);margin-bottom:5px}.certificate-task-metrics strong{display:block;color:var(--text,#222);font-size:19px;font-variant-numeric:tabular-nums;line-height:1.4;overflow-wrap:anywhere}.certificate-task-metrics strong small{font-weight:400;font-size:12px;color:var(--muted)}.certificate-task-metrics .warning{color:#a96c14}.certificate-task-panel progress{display:block;width:100%;height:6px;accent-color:var(--blue,#2878ff);margin:12px 0}.certificate-task-panel .certificate-task-note{font-size:11px;overflow-wrap:anywhere}.certificate-task-details{border-top:1px solid var(--line);padding-top:12px;margin-top:14px}.certificate-task-details summary{cursor:pointer;color:var(--muted);font-size:11px}.certificate-task-details p{font-size:11px;overflow-wrap:anywhere;margin-top:10px}.certificate-task-table{max-height:260px;overflow:auto;margin:10px 0}table{border-collapse:collapse;width:100%;font-size:12px}th,td{text-align:left;padding:8px;border-bottom:1px solid var(--line);overflow-wrap:anywhere;max-width:360px}.certificate-task-panel .certificate-task-empty{padding:12px 0;font-size:12px}.certificate-task-panel .certificate-task-error{color:var(--danger,#c33);margin-top:12px;overflow-wrap:anywhere}@media(max-width:1000px){.certificate-task-metrics{grid-template-columns:repeat(3,minmax(0,1fr))}.certificate-task-actions{margin-left:0}.task-history{max-width:none}}@media(max-width:600px){.certificate-task-panel{padding:14px}.certificate-task-heading{flex-wrap:wrap;gap:10px}.certificate-task-heading .certificate-refresh{margin-left:44px}.certificate-task-scope{grid-template-columns:1fr;gap:7px}.certificate-task-scope>small{grid-column:1}.certificate-task-toolbar{gap:10px}.task-history{flex-basis:100%}.certificate-task-actions{width:100%;align-items:stretch}.certificate-task-actions .button{flex:1 1 auto;white-space:normal}.certificate-task-metrics{grid-template-columns:repeat(2,minmax(0,1fr))}.certificate-task-progress-heading{flex-wrap:wrap}.task-record-label>strong{line-height:1.6}}
</style>
