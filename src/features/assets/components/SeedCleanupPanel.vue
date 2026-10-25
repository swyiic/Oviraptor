<script setup lang="ts">
import {computed,onUnmounted,ref,watch} from 'vue';
import {Play,Pause,RotateCcw,RefreshCw,ChevronLeft,ChevronRight} from '@lucide/vue';
import {assetApi} from '../api';
import {useI18n} from '../uiPorts';
import {refreshFeedback as vRefreshFeedback} from '../../../shared/directives/refreshFeedback';
import type {SeedCleanup} from '../seeds/cleanup';
const props=defineProps<{projectId:number}>();
const emit=defineEmits<{changed:[]}>();
const {tr}=useI18n();
const data=ref<SeedCleanup>(),organization=ref(''),busy=ref(false),reading=ref(false),error=ref(''),uncertain=ref(false),filter=ref('conflict'),page=ref(1);
let pendingOperation='',generation=0,sequence=0,disposed=false,timer:ReturnType<typeof setTimeout>|undefined;
const runId=computed(()=>data.value?.runId||data.value?.id);
const enabled=computed(()=>data.value?.enabled===true);
const counts=computed(()=>data.value?.counts||{});
const percent=computed(()=>{const s=data.value?.snapshot;return s?.total?Math.min(100,Math.round((s.processed||0)*100/s.total)):0;});
const pages=computed(()=>Math.max(1,Math.ceil((data.value?.total||0)/25)));
const labels:Record<string,[string,string]>={preparing:['整理全项目种子','Preparing all project seeds'],researching:['自动分析与联网补证','Researching evidence'],waiting_evidence:['等待补证重试','Waiting for evidence retry'],paused:['已暂停，可继续','Paused; resumable'],analyzing:['AI 正在分析证据','AI evidence analysis'],completed:['本轮已完成','Round completed'],excluded:['已自动排除','Automatically excluded'],retained:['保留相关线索','Related leads retained'],evidence:['自动继续补证','Continuing evidence research'],held:['保留，等待关联证据','Retained pending related evidence'],needs_evidence:['自动继续补证','Continuing evidence research'],conflict:['证据冲突','Evidence conflict'],idle:['尚未开始','Not started']};
function label(value?:string){return labels[value||'idle']?tr(...labels[value||'idle']):value;}
function number(value?:number){return typeof value==='number'?value.toLocaleString():tr('待统计','Pending');}
function schedule(){clearTimeout(timer);if(enabled.value&&!disposed)timer=setTimeout(()=>void read(),2500);}
function accept(value:SeedCleanup,p:number,g:number,seq:number){
 if(disposed||g!==generation||seq!==sequence||p!==props.projectId)return false;
 if(value.projectId!==p||value.items!==undefined&&!Array.isArray(value.items))throw Error('seed_cleanup_scope_mismatch');
 data.value=value;if(pendingOperation&&value.operation?.id===pendingOperation&&['completed','rejected'].includes(value.operation.state)){uncertain.value=false;pendingOperation='';}if(!organization.value.trim())organization.value=value.organization||'';return true;
}
async function read(){
 const p=props.projectId,g=generation,seq=++sequence;reading.value=true;
 try{const value=await assetApi.seedCleanup(p,'status',{...(runId.value?{runId:runId.value}:{}),...(pendingOperation?{operationId:pendingOperation}:{}),page:page.value,pageSize:25,filter:filter.value},`read-${p}`);if(accept(value,p,g,seq)){error.value='';schedule();}}
 catch(e){if(!disposed&&g===generation&&seq===sequence)error.value=String(e);}
 finally{if(!disposed&&g===generation&&seq===sequence)reading.value=false;}
}
async function control(action:'start'|'pause'|'resume'|'restore'){
 if(busy.value||uncertain.value||action==='start'&&!organization.value.trim())return;
 const p=props.projectId,g=generation,seq=++sequence;
 busy.value=true;error.value='';clearTimeout(timer);
 const operationId=globalThis.crypto.randomUUID();pendingOperation=operationId;
 const args=action==='start'?{organization:organization.value.trim(),autoExcludeUnrelated:true}:{runId:runId.value};
 try{const value=await assetApi.seedCleanup(p,action,args,operationId);if(accept(value,p,g,seq)){uncertain.value=value.outcomeUnknown===true;if(!uncertain.value)pendingOperation='';emit('changed');schedule();}}
 catch(e){if(!disposed&&g===generation&&seq===sequence){error.value=String(e);uncertain.value=!/^cleanup_(?:arguments_invalid|organization_invalid|project_already_running|run_project_mismatch|filter_invalid|action_invalid|trusted_operation_required)$/.test(String(e));if(!uncertain.value)pendingOperation='';}}
 finally{if(!disposed&&g===generation&&seq===sequence)busy.value=false;}
}
async function changePage(next:number){if(reading.value||busy.value)return;const old=page.value;page.value=Math.max(1,Math.min(pages.value,next));await read();if(error.value)page.value=old;}
watch(filter,()=>{page.value=1;void read();});
watch(()=>props.projectId,()=>{pendingOperation='';generation++;sequence++;clearTimeout(timer);data.value=undefined;organization.value='';busy.value=false;reading.value=false;error.value='';uncertain.value=false;page.value=1;void read();},{immediate:true,flush:'sync'});
onUnmounted(()=>{disposed=true;generation++;sequence++;clearTimeout(timer);});
</script>
<template>
<section class="cleanup-panel" data-testid="seed-cleanup-panel" :aria-busy="busy||reading">
 <header><div><h3>{{tr('AI 清洗项目种子','AI project seed cleanup')}}</h3><p>{{tr('全量整理 → 联网核对 → 明确无关自动排除；缺证继续查，冲突集中看。','Read all seeds, verify evidence online, exclude proven unrelated groups, research missing evidence and group conflicts.')}}</p></div><button v-refresh-feedback="{scope:projectId,busy:reading,error}" class="button ghost compact" :disabled="busy||reading" data-cleanup-action="refresh" @click="read"><RefreshCw :size="14"/>{{tr('刷新进度','Refresh progress')}}</button></header>
 <div class="cleanup-start"><label>{{tr('目标单位与业务范围','Target organization and scope')}}<input v-model="organization" maxlength="240" :disabled="enabled||busy||!!runId" :placeholder="tr('例如：中国移动集团及其下属单位','For example: organization and subsidiaries')"/></label><button v-if="!runId" class="button primary compact" :disabled="busy||uncertain||!organization.trim()" data-cleanup-action="start" @click="control('start')"><Play :size="14"/>{{tr('开始自动清洗','Start cleanup')}}</button><button v-else-if="enabled" class="button ghost compact" :disabled="busy||uncertain" data-cleanup-action="pause" @click="control('pause')"><Pause :size="14"/>{{tr('暂停','Pause')}}</button><button v-else class="button primary compact" :disabled="busy||uncertain" data-cleanup-action="resume" @click="control('resume')"><Play :size="14"/>{{tr('继续本轮清洗','Continue this cleanup')}}</button></div>
 <p class="cleanup-policy">{{tr('保留原始资产、种子和历史，不改变已授权探测范围，不调用 Hunter。AI 意见必须结合域名对应的事实证据，排除操作按批记录，可撤销。','Keeps original assets, seeds and history, preserves probing scope and does not use Hunter. AI opinions require domain-bound evidence; exclusions are recorded and reversible.')}}</p>
 <template v-if="data"><div class="cleanup-state" role="status"><strong>{{label(data.phase)}}</strong><span>{{data.reason}}</span></div><progress v-if="data.phase==='preparing'" :value="percent" max="100" :aria-label="tr('全量种子整理进度','Whole-project seed preparation')"/><div class="cleanup-metrics"><span>{{tr('原始索引','Raw index')}} <b>{{number(data.snapshot?.rawIndexSeeds)}}</b></span><span>{{tr('项目台账','Project ledger')}} <b>{{number(data.snapshot?.projectLedgerSeeds)}}</b></span><span>{{tr('证据组','Evidence groups')}} <b>{{number(data.snapshot?.groups)}}</b></span><span>{{tr('排除组','Excluded groups')}} <b>{{number(counts.excluded||0)}}</b></span><span>{{tr('补证组','Research groups')}} <b>{{number(counts.needs_evidence||counts.evidence||0)}}</b></span><span>{{tr('冲突组','Conflict groups')}} <b>{{number(counts.conflict||0)}}</b></span></div>
 <p v-if="runId" class="cleanup-policy">{{tr('公开补证已完成','Public research completed')}} {{number(data.research?.completed||0)}} · {{tr('排队或重试','Queued or retrying')}} {{number(data.research?.pending||0)}} · {{tr('模型请求','Model requests')}} {{number(data.model?.requests||0)}} · Token {{number(data.model?.inputTokens||0)}} / {{number(data.model?.outputTokens||0)}}<span v-if="data.model?.unknownRequests"> · {{tr('结果未知，不重放','Unconfirmed, never replayed')}} {{data.model.unknownRequests}}</span></p>
 <div v-if="runId" class="cleanup-results"><div class="cleanup-results-heading"><select v-model="filter" :disabled="busy" :aria-label="tr('清洗结果','Cleanup results')"><option value="all">{{tr('全部处理结果','All results')}}</option><option value="conflict">{{tr('需要决定的冲突','Conflicts requiring a decision')}}</option><option value="excluded">{{tr('已自动排除','Automatically excluded')}}</option><option value="needs_evidence">{{tr('系统继续补证','Evidence research queue')}}</option><option value="retained">{{tr('保留的线索','Retained leads')}}</option></select><button class="button ghost compact" :disabled="busy||uncertain||enabled||!data.batches?.some(b=>!b.restored&&!b.restoredAt)" data-cleanup-action="restore" @click="control('restore')"><RotateCcw :size="14"/>{{tr('撤销本轮排除','Undo this cleanup’s exclusions')}}</button></div>
 <article v-for="(item,index) in data.items||[]" :key="item.key||item.domain||index"><div><b>{{item.domain||item.key}}</b><span :class="item.status||item.state">{{label(item.status||item.state)}}</span></div><p>{{item.reason}}</p><details v-if="item.evidence||item.sources?.length"><summary>{{tr('查看核对依据','Verification evidence')}}</summary><pre>{{JSON.stringify(item.evidence||item.sources,null,2)}}</pre></details></article><p v-if="!data.items?.length" class="cleanup-policy">{{reading?tr('正在读取…','Loading…'):tr('此筛选下暂无结果。未完成的补证会保留断点。','No results for this filter. Unfinished evidence research retains its checkpoint.')}}</p><footer><span>{{page}} / {{pages}}</span><button class="button ghost compact" :disabled="reading||busy||page<=1" data-cleanup-action="previous" @click="changePage(page-1)"><ChevronLeft :size="14"/></button><button class="button ghost compact" :disabled="reading||busy||page>=pages" data-cleanup-action="next" @click="changePage(page+1)"><ChevronRight :size="14"/></button></footer></div></template>
 <p v-if="error" class="cleanup-error" role="alert">{{error}}</p><p v-if="uncertain" class="cleanup-error">{{tr('操作结果尚未确认，已阻止重复提交。可刷新进度核对，原始数据保留。','The operation outcome is unconfirmed; repeat submission is blocked. Refresh progress to reconcile it. Original data is retained.')}}</p>
</section>
</template>
<style scoped>
.cleanup-panel{padding:16px;border:1px solid var(--line);border-radius:10px;margin:0 0 16px;background:var(--card,#fff);min-width:0}.cleanup-panel header,.cleanup-start,.cleanup-results-heading,.cleanup-state,.cleanup-metrics,.cleanup-results footer{display:flex;align-items:center;gap:10px;flex-wrap:wrap}.cleanup-panel header{justify-content:space-between;align-items:flex-start}.cleanup-panel h3{margin:0;font-size:14px}.cleanup-panel header p,.cleanup-policy{font-size:11px;color:var(--muted);line-height:1.65;margin:5px 0 12px}.cleanup-start{margin-top:12px;align-items:flex-end}.cleanup-start label{display:grid;gap:6px;flex:1 1 280px;font-size:11px;min-width:0}.cleanup-start input{width:100%;box-sizing:border-box;min-height:36px;padding:8px 10px}.cleanup-panel button{white-space:normal;min-height:34px}.cleanup-state{margin-top:12px;font-size:12px}.cleanup-state span{color:var(--muted);overflow-wrap:anywhere}.cleanup-panel progress{width:100%;height:6px;margin-top:12px;accent-color:var(--accent,#2874ff)}.cleanup-metrics{margin:12px 0;font-size:11px;gap:6px}.cleanup-metrics>span{border:1px solid var(--line);padding:7px 10px;border-radius:6px;color:var(--muted)}.cleanup-metrics b{margin-left:5px;color:var(--text)}.cleanup-results{border-top:1px solid var(--line);padding-top:10px}.cleanup-results-heading{justify-content:space-between}.cleanup-results-heading select{min-height:34px;max-width:100%}.cleanup-results article{border-top:1px solid var(--line);padding:11px 0;font-size:11px;margin-top:10px;overflow-wrap:anywhere}.cleanup-results article>div{display:flex;justify-content:space-between;gap:10px;flex-wrap:wrap}.cleanup-results article p{color:var(--muted);margin:5px 0}.cleanup-results details{color:var(--muted)}.cleanup-results summary{cursor:pointer}.cleanup-results pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:260px;overflow:auto}.cleanup-results footer{justify-content:flex-end;font-size:11px;margin-top:8px}.cleanup-results footer>span{margin-right:auto}.cleanup-error{color:#a04733;font-size:11px;overflow-wrap:anywhere}.excluded{color:#7b6262}.conflict{color:#b56d00}.retained{color:#258365}@media(max-width:680px){.cleanup-panel{padding:12px}.cleanup-start>button{width:100%}.cleanup-results-heading{align-items:stretch}.cleanup-results-heading>*{flex:1 1 200px}}
</style>
