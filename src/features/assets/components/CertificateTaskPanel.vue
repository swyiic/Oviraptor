<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { Play, Pause, RefreshCw, ShieldCheck } from '@lucide/vue';
const props=defineProps<{projectId?:number;archived?:boolean}>();
const emit=defineEmits<{changed:[]}>();
type Progress={total:number;captured:number;failed:number;skipped:number;pending:number;running:number;uniqueFingerprints:number;newFingerprints:number;attempts:number};
type Task={createdAt?:string;selection?:{roots?:string[];httpsAssets:number;outsideConfirmedScope:number;alreadyHaveFingerprint:number;queued:number};id:string;status:string;reason:string;workers:number;progress:Progress;items:{assetId:number;endpoint:string;status:string;reason:string;attempts:number;fingerprint:string}[]};
type View={roots:string[];tasks:Task[]};
const data=ref<View>(),busy=ref(false),error=ref(''),workers=ref(4),expanded=ref(false),taskId=ref('');
let generation=0,disposed=false,timer:ReturnType<typeof setTimeout>|undefined;
const task=computed(()=>data.value?.tasks.find(t=>t.id===taskId.value)??data.value?.tasks[0]);
const active=computed(()=>task.value&&['running','pausing'].includes(task.value.status));
const done=computed(()=>task.value?task.value.progress.captured+task.value.progress.failed+task.value.progress.skipped:0);
const percent=computed(()=>task.value?.progress.total?Math.floor(done.value/task.value.progress.total*100):0);
const labels:Record<string,string>={running:'正在补采',pausing:'正在保存并暂停',paused:'已暂停',interrupted:'应用退出，等待恢复',completed:'处理完成',failed:'执行中断，请检查原因',pending:'待处理',captured:'证书已保存',skipped:'已跳过'};
async function load(g=generation){
 const projectId=props.projectId;if(!projectId||disposed)return;
 try{const value=await invoke<View>('certificate_task_action',{projectId,action:'status',args:{}});if(disposed||g!==generation)return;const old=task.value;data.value=value;if(task.value&&!['completed','failed'].includes(task.value.status))workers.value=task.value.workers;if(!value.tasks.some(t=>t.id===taskId.value))taskId.value=value.tasks[0]?.id??'';error.value='';if(old?.status==='running'&&task.value?.status!=='running')emit('changed');}
 catch(e){if(g===generation&&!disposed)error.value=String(e);}
 finally{clearTimeout(timer);if(!disposed&&g===generation&&data.value?.tasks.some(t=>['running','pausing'].includes(t.status)))timer=setTimeout(()=>void load(g),1500);}
}
async function act(action:string,retryFailed=false){
 if(!props.projectId||busy.value||props.archived)return;const projectId=props.projectId,g=generation;busy.value=true;error.value='';
 try{const result=await invoke<View>('certificate_task_action',{projectId,action,args:action==='start'?{workers:workers.value,timeoutSeconds:8}:{taskId:task.value?.id,retryFailed}});if(g===generation&&!disposed){taskId.value=result.tasks[0]?.id??taskId.value;expanded.value=true;await load(g);emit('changed');}}
 catch(e){if(g===generation&&!disposed)error.value=String(e);}finally{if(g===generation&&!disposed)busy.value=false;}
}
watch(()=>props.projectId,()=>{generation++;clearTimeout(timer);data.value=undefined;taskId.value='';error.value='';busy.value=false;void load();},{immediate:true});
onUnmounted(()=>{disposed=true;generation++;clearTimeout(timer);});
</script>
<template>
 <section class="certificate-task-panel">
  <header><div><h3><ShieldCheck :size="17"/>读取缺失的 TLS 证书</h3><p>给本地已收录、确认域名范围内的 HTTPS 资产补齐证书 SHA-256。连接目标读取证书，不获取网页，不调用 FOFA / Hunter。</p></div><button class="button secondary compact" :disabled="busy||!projectId" @click="load()"><RefreshCw :size="14"/>刷新进度</button></header>
  <div class="certificate-task-scope">新任务确认范围：<strong>{{data?.roots.length?data.roots.join('、'):'尚未设置，请先到「新建查询 → 域名线索与补查」保存范围'}}</strong><small>只按这些域名选择资产，不代表覆盖整个集团；IP 入口不会仅凭证书关联自动加入握手队列。</small></div>
  <div class="certificate-task-actions">
   <label v-if="data&&data.tasks.length>1" class="task-history">查看任务记录<select v-model="taskId" aria-label="证书补采任务历史"><option v-for="item in data.tasks" :key="item.id" :value="item.id">{{item.createdAt||item.id.slice(0,8)}} · {{labels[item.status]||item.status}}</option></select></label>
   <label v-if="!task||['completed','failed'].includes(task.status)">下次同时处理资产数<select v-model.number="workers" :disabled="busy"><option :value="1">1 台</option><option :value="4">4 台</option><option :value="8">8 台</option><option :value="16">16 台</option></select></label>
   <button v-if="!task||task.status==='completed'" class="button primary" :disabled="busy||!projectId||archived||!data?.roots.length" @click="act('start')"><Play :size="14"/>{{task?'检查并补采剩余证书':'开始补采缺失证书'}}</button>
   <button v-if="task&&['running','pausing'].includes(task.status)" class="button" :disabled="busy||task.status==='pausing'||archived" @click="act('pause')"><Pause :size="14"/>{{task.status==='pausing'?'正在暂停…':'暂停补采'}}</button>
   <button v-if="task&&['paused','interrupted','failed'].includes(task.status)" class="button primary" :disabled="busy||archived" @click="act('resume')"><Play :size="14"/>继续未完成项</button>
   <button v-if="task&&!active&&task.progress.failed>0" class="button" :disabled="busy||archived" @click="act('resume',true)">重试失败项 · {{task.progress.failed}}</button>
   <span v-if="task" class="certificate-task-status">{{labels[task.status]||task.status}}<template v-if="active"> · 最多同时读取 {{task.workers}} 台</template></span>
  </div>
  <template v-if="task">
   <p v-if="task.selection?.roots" class="certificate-task-note">此任务创建时的范围：{{task.selection.roots.join('、')}}</p>
   <p v-if="task.selection" class="certificate-task-note">创建时有 {{task.selection.httpsAssets.toLocaleString()}} 条 HTTPS 记录：确认范围外 {{task.selection.outsideConfirmedScope.toLocaleString()}} 条，范围内已有指纹 {{task.selection.alreadyHaveFingerprint.toLocaleString()}} 条，本任务待补采 {{task.selection.queued.toLocaleString()}} 条。</p>
   <div class="certificate-task-metrics"><span>资产进度 <strong>{{done.toLocaleString()}} / {{task.progress.total.toLocaleString()}}</strong></span><span>证书已保存 <strong>{{task.progress.captured.toLocaleString()}}</strong></span><span>失败 <strong>{{task.progress.failed.toLocaleString()}}</strong></span><span>跳过 <strong>{{task.progress.skipped.toLocaleString()}}</strong></span><span>唯一指纹 <strong>{{task.progress.uniqueFingerprints.toLocaleString()}}</strong></span><span>新增指纹 <strong>{{task.progress.newFingerprints.toLocaleString()}}</strong></span></div>
   <progress :value="done" :max="Math.max(1,task.progress.total)" :aria-label="`证书补采进度 ${percent}%`"/>
   <p class="certificate-task-note">{{task.reason}}。同一张证书部署在多台资产上，只计一个唯一指纹。补采完成不会自动查询 Hunter，也不会解除隔离；付费补查还会检查证书中的域名。</p>
   <details :open="expanded" @toggle="expanded=($event.target as HTMLDetailsElement).open"><summary>任务详情 · {{task.id.slice(0,8)}}</summary><div class="certificate-task-table"><table><thead><tr><th>资产</th><th>状态</th><th>说明</th></tr></thead><tbody><tr v-for="item in task.items" :key="item.assetId"><td>{{item.endpoint}}</td><td>{{labels[item.status]||item.status}}</td><td>{{item.reason||'等待执行'}}</td></tr></tbody></table></div><small>最多展示 20 条；完整进度由任务记录保存。暂停会先保存当前握手结果。</small></details>
  </template>
  <p v-if="error" class="certificate-task-error" role="alert">{{error}}</p>
 </section>
</template>
<style scoped>
.certificate-task-panel{margin:0 0 18px;padding:20px;border:1px solid var(--line,#e5e5e5);border-radius:12px;background:var(--app-surface,#fff);font-size:13px;line-height:1.6;min-width:0}.certificate-task-panel header{display:flex;justify-content:space-between;gap:16px;align-items:flex-start}.certificate-task-panel h3{display:flex;gap:8px;align-items:center;font-size:15px;margin:0 0 6px}.certificate-task-panel p{margin:0;color:var(--muted,#777);font-size:12px}.certificate-task-scope{margin-top:12px;font-size:12px;overflow-wrap:anywhere}.certificate-task-actions{display:flex;align-items:center;flex-wrap:wrap;gap:10px;margin-top:14px}.certificate-task-actions label{display:flex;flex-direction:column;align-items:flex-start;gap:5px;font-size:12px;color:var(--muted)}.certificate-task-panel .button{display:inline-flex;align-items:center;justify-content:center;gap:7px;white-space:nowrap;min-height:36px;padding:8px 12px;font:inherit}.certificate-task-actions .task-history select{min-width:210px;max-width:100%}.certificate-task-scope small{display:block;margin-top:5px}.certificate-task-actions{align-items:flex-end}.certificate-task-metrics{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:10px}.certificate-task-metrics>span{display:flex;justify-content:space-between;padding:10px;border:1px solid var(--line);border-radius:8px}.certificate-task-panel .certificate-task-note{margin-top:10px}.certificate-task-actions select{min-width:65px;padding:6px;border:1px solid var(--line);border-radius:6px;background:var(--app-surface);color:inherit}.certificate-task-status{color:var(--muted);font-size:12px}.certificate-task-metrics{display:flex;flex-wrap:wrap;gap:12px 22px;margin-top:16px;color:var(--muted);font-size:12px}.certificate-task-metrics strong{color:var(--text,#222);margin-left:5px}.certificate-task-panel progress{display:block;width:100%;height:8px;accent-color:var(--blue,#2878ff);margin:12px 0}.certificate-task-panel .certificate-task-note{font-size:11px}.certificate-task-panel details{margin-top:12px}.certificate-task-panel summary{cursor:pointer;color:var(--muted);font-size:12px}.certificate-task-table{max-height:260px;overflow:auto;margin:8px 0}table{border-collapse:collapse;width:100%;font-size:12px}th,td{text-align:left;padding:8px;border-bottom:1px solid var(--line);overflow-wrap:anywhere}small{color:var(--muted);font-size:11px}.certificate-task-panel .certificate-task-error{color:var(--danger,#c33);margin-top:12px}@media(max-width:600px){.certificate-task-metrics{grid-template-columns:repeat(2,minmax(0,1fr))}.certificate-task-panel{padding:14px}.certificate-task-panel header{flex-direction:column}}
</style>
