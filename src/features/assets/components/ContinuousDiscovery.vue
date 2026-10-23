<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import {computed,onMounted,onUnmounted,ref,watch} from 'vue';
import {invoke} from '../uiPorts';
import {Play,Pause,RefreshCw} from '@lucide/vue';
const props=defineProps<{projectId?:number;refreshVersion?:string;archived?:boolean}>();
const emit=defineEmits<{changed:[]}>();
type Conflict={kind:string;identity:string;hosts:string[];hostCount?:number;detail:string;decision:string;revision:number};
type View={roots:string[];run:null|{id:string;status:string;enabled:boolean;continuous:boolean;intervalSeconds:number;workers:number;counts:Record<string,number>;operations:number;reason:string;nextCheckAt?:number};events:{kind:string;target:string;outcome:string;detail:string;at:number}[];conflicts:Conflict[]};
const data=ref<View>(),error=ref(''),readError=ref(''),busy=ref(false),continuous=ref(true),interval=ref(60),workers=ref(4);
let generation=0,sequence=0,disposed=false,timer:ReturnType<typeof setTimeout>|undefined,signature='';
let readFlight:{generation:number;sequence:number;promise:Promise<void>}|undefined;
const run=computed(()=>data.value?.run),active=computed(()=>!!run.value&&['running','waiting','waiting_storage','pausing'].includes(run.value.status));
const conflicts=computed(()=>data.value?.conflicts.filter(c=>c.decision==='pending')||[]);
const states:Record<string,string>={running:'正在补证',waiting:'持续监测中',waiting_storage:'等待本地存储，自动重试',pausing:'正在保存并暂停',paused:'已暂停，可继续',completed:'本轮已完成',interrupted:'已中断，可继续',scope_changed:'范围变化，需重新启动'};
const kinds:Record<string,string>={ct:'CT',dns_a:'DNS A',dns_aaaa:'DNS AAAA',dns_cname:'CNAME',tls:'TLS 证书'};
function schedule(g=generation){clearTimeout(timer);if(!disposed&&!props.archived&&!document.hidden&&g===generation&&active.value)timer=setTimeout(()=>void load(g),run.value?.status==='waiting'?10000:2500);}
function load(g=generation):Promise<void>{
 const p=props.projectId;if(!p||disposed||g!==generation||busy.value||document.hidden)return Promise.resolve();
 if(readFlight?.generation===g&&readFlight.sequence===sequence)return readFlight.promise;
 const seq=++sequence,flight={generation:g,sequence:seq,promise:Promise.resolve()};
 flight.promise=(async()=>{try{const v=await invoke<View>('enrichment_action',{projectId:p,action:'status',args:{}});if(disposed||g!==generation||seq!==sequence)return;data.value=v;readError.value='';const next=JSON.stringify([v.run?.id,v.run?.counts?.done,v.events?.[0]?.at,v.events?.[0]?.target]);if(signature&&signature!==next)emit('changed');signature=next;}catch(e){if(!disposed&&g===generation&&seq===sequence)readError.value=String(e);}finally{if(readFlight===flight)readFlight=undefined;if(g===generation&&seq===sequence)schedule(g);}})();
 readFlight=flight;return flight.promise;
}
async function control(action:string,conflict?:Conflict){if(busy.value||props.archived||!props.projectId||disposed)return;const g=generation;sequence++;readFlight=undefined;clearTimeout(timer);busy.value=true;error.value='';let failed=false;try{const args=action==='start'?{continuous:continuous.value,intervalSeconds:interval.value*60,workers:workers.value}:conflict?{kind:conflict.kind,identity:conflict.identity,expectedRevision:conflict.revision,decision:'acknowledged'}:{taskId:run.value?.id};const value=await invoke<View>('enrichment_action',{projectId:props.projectId,action,args});if(g!==generation||disposed)return;data.value=value;readError.value='';emit('changed');}catch(e){if(g===generation&&!disposed){error.value=String(e);failed=true;}}finally{if(g===generation&&!disposed){busy.value=false;if(failed)void load(g);else schedule(g);}}}
watch(()=>props.projectId,()=>{generation++;sequence++;clearTimeout(timer);data.value=undefined;busy.value=false;error.value='';readError.value='';signature='';void load();},{immediate:true});
watch(()=>props.refreshVersion,()=>void load());
function visibilityChanged(){if(document.hidden)clearTimeout(timer);else void load();}
onMounted(()=>document.addEventListener('visibilitychange',visibilityChanged));
onUnmounted(()=>{disposed=true;generation++;readFlight=undefined;clearTimeout(timer);document.removeEventListener('visibilitychange',visibilityChanged);});
</script>
<template>
<section class="continuous-discovery" aria-label="自动补证与持续发现">
<header><div><h4>自动补证与持续发现</h4><p>本地证书 → CT 新名称 → DNS / CNAME → TLS 证书 → 新种子，自动循环并逐项保存。</p></div><span class="status">{{archived?'已归档，只读':run?states[run.status]||run.status:'尚未启动'}}</span></header>
<p class="scope">已确认范围：<strong>{{data?.roots?.join('、')||'尚未设置，请先保存确认域名范围'}}</strong></p>
<div v-if="!archived&&(!run||['completed','scope_changed'].includes(run.status))" class="controls">
<label>检查间隔<select v-model.number="interval" :disabled="busy"><option :value="15">15 分钟</option><option :value="60">1 小时</option><option :value="360">6 小时</option><option :value="1440">24 小时</option></select></label>
<label>并发<select v-model.number="workers" :disabled="busy"><option v-for="n in [1,2,4,8]" :key="n" :value="n">{{n}}</option></select></label>
<label class="check"><input v-model="continuous" type="checkbox" :disabled="busy"/>持续检查新线索（运行中重启后继续）</label>
<button class="button primary compact" :disabled="busy||!data?.roots?.length" @click="control('start')"><Play :size="14"/>开始自动补证</button>
</div>
<div v-else-if="!archived&&run" class="controls"><button v-if="active" class="button secondary compact" :disabled="busy||run.status==='pausing'" @click="control('pause')"><Pause :size="14"/>{{run.status==='pausing'?'正在暂停…':'暂停自动补证'}}</button><button v-else class="button primary compact" :disabled="busy" @click="control('resume')"><Play :size="14"/>从断点继续</button><span>{{run.continuous?'持续发现':'单轮补证'}} · 并发 {{run.workers}} · 间隔 {{run.intervalSeconds/60}} 分钟</span></div>
<div v-if="run" class="progress" role="status"><span>已完成 <b>{{run.counts.done||0}}</b></span><span>排队 <b>{{run.counts.pending||0}}</b></span><span>执行 <b>{{run.counts.running||0}}</b></span><span>错误待重试 <b>{{run.counts.retry||0}}</b></span><span>失败待处理 <b>{{run.counts.failed||0}}</b></span><span>已跳过 <b>{{run.counts.skipped||0}}</b></span><span>Hunter <b>0 积分</b></span></div>
<p v-if="run" class="reason">{{run.reason}} <small>任务 {{run.id.slice(0,8)}}</small></p>
<p class="explanation">仅处理已确认范围。范围外 CNAME 保留关系；通配符不是已发现主机。自动处理缺证与网络重试；证书身份冲突才汇总复核。不会恢复隔离 / 回收资产，不读取页面，不调用 Hunter 或逐条调用模型。CT / DNS 使用现有网络配置；TLS 证书每日复查。</p>
<details v-if="conflicts.length"><summary>需要复核的身份冲突 · {{conflicts.length}} 组（最多预览 20 组）</summary><article v-for="c in conflicts" :key="c.kind+c.identity"><code>{{c.identity}}</code><p>{{c.hosts.join('、')}}<span v-if="c.hostCount&&c.hostCount>c.hosts.length"> 等 {{c.hostCount}} 个主机</span></p><p>{{c.detail}}</p><button class="button secondary compact" :disabled="busy||archived" @click="control('review_conflict',c)">已阅，保留证据</button></article></details>
<details v-if="data?.events?.length"><summary>最近执行与详细错误</summary><article v-for="(e,i) in data.events" :key="i" :class="{failed:e.outcome==='error'||e.outcome==='failed'}"><strong>{{kinds[e.kind]||e.kind}} · {{e.outcome==='failed'?'永久错误或已达到重试上限':e.outcome==='error'?'未成功，退避重试':'已保存'}}</strong><code>{{e.target}}</code><p>{{e.detail}}</p><small>{{new Date(e.at*1000).toLocaleString()}}</small></article></details>
<footer><span>缺少证据不等于无关，查询失败不等于资产不存在。</span><button v-refresh-feedback="{scope:projectId,error:readError}" class="button ghost compact" :disabled="busy" @click="load()"><RefreshCw :size="13"/>刷新进度</button></footer>
<p v-if="error" role="alert" class="form-error">{{error}}</p>
<p v-if="readError" role="alert" class="form-error">{{readError}}</p>
</section>
</template>
<style scoped>
.continuous-discovery{border:1px solid var(--line);border-radius:9px;padding:18px;margin-bottom:16px;min-width:0;background:var(--app-surface,#fff);font-size:12px;line-height:1.7}.continuous-discovery header{display:flex;justify-content:space-between;gap:12px;flex-wrap:wrap;align-items:flex-start}h4{font-size:14px;margin:0 0 5px}p{margin:6px 0;color:var(--muted)}.status{padding:2px 8px;background:var(--app-bg,#fafafa);border:1px solid var(--line);border-radius:5px;white-space:nowrap}.scope{overflow-wrap:anywhere}.scope strong{color:var(--text)}.controls{display:flex;align-items:flex-end;flex-wrap:wrap;gap:12px;margin:14px 0}.controls label{display:flex;flex-direction:column;gap:5px;min-width:85px}.controls select{width:100%;min-height:34px;font-size:12px}.controls .check{flex-direction:row;align-items:center;gap:7px;min-height:34px}.check input{width:15px;height:15px;min-height:0;flex:0 0 15px}.progress{display:flex;gap:8px 20px;flex-wrap:wrap;padding:10px 12px;background:var(--app-bg,#fafafa);border:1px solid var(--line);border-radius:6px}.progress b{margin-left:5px}.reason small{margin-left:10px}.explanation{font-size:11px}details{margin-top:12px}summary{cursor:pointer;color:var(--muted)}article{padding:10px 0;border-bottom:1px solid var(--line);overflow-wrap:anywhere}code{display:block;font-size:11px;overflow-wrap:anywhere}.failed strong{color:#a86b16}footer{display:flex;justify-content:space-between;gap:10px;align-items:center;flex-wrap:wrap;margin-top:10px;color:var(--muted);font-size:11px}@media(max-width:600px){.continuous-discovery{padding:14px}.controls>.button{width:100%}.controls .check{flex-basis:100%}}
</style>
