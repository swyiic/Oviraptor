<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import {computed,onUnmounted,ref,watch} from 'vue';
import {Bot,Play,Pause,RefreshCw,Check,ChevronLeft,ChevronRight,ArrowRight} from '@lucide/vue';
import {invoke} from '../uiPorts';
import type {DiscoveryPipeline,DiscoveryVerification} from '../../agentHost/discoveryTypes';
import DiscoveryPipelineProgress from './DiscoveryPipelineProgress.vue';
const props=defineProps<{projectId?:number;refreshVersion?:number}>();
const emit=defineEmits<{confirmed:[];collect:[];results:[]}>();
type Evidence={domains:string[];companies:string[];subjectOrganizations:string[];organizationHints?:string[];icpOrganizations?:string[];sources:string[];exampleAssetIds:number[];quarantinedAssets:number;ambiguousIdentity:boolean;evidenceTruncated:boolean};
type Item={key:string;kind:string;target:string;assetCount:number;state:string;advice:string;confidence:number;reason:string;queueReason?:string;decision:string;revision:number;evidence:Evidence};
type ReviewRun={includeIsolated?:boolean;id:string;organization:string;status:string;totalAssets:number;processedAssets:number;cursor:number;maxId:number;groups:number;counts:Record<string,number>;queueCounts:Record<string,number>;manualCounts:Record<string,number>;model:string;requests:number;inputTokens:number;outputTokens:number;unknownRequests:number;reason:string};
type View={run:ReviewRun|null;items:Item[];total:number;roots?:string[]};
type ScopeGroup={root:string;shared:boolean;scopes:string[];observedDomains:string[];observedCount:number;evidenceGroups:number;certificateGroups:number;pendingEvidenceGroups:number;conflictGroups:number;assetCount:number;reasons:string[];verification?:DiscoveryVerification};
type Proposal={token:string;roots:string[];groups:ScopeGroup[];totalEvidenceGroups:number;removedNoLiveEvidence?:number;heldForVerification?:number;removedUnverified?:number;verificationPolicyVersion?:number;networkVerified?:boolean};
type Assist={phase:string;enabled:boolean;reason:string;runId?:string;organization?:string;hasLocalAssets:boolean;review:View;proposal:Proposal|null;enrichmentError?:string;pipeline?:DiscoveryPipeline|null};
type EnrichmentConflict={kind:string;identity:string;hosts:string[];hostCount?:number;detail:string;decision:string;revision:number};
type Enrichment={roots:string[];conflicts?:EnrichmentConflict[];run:null|{id:string;status:string;enabled:boolean;counts:Record<string,number>;reason:string};hunterRequests?:number};
const assist=ref<Assist>(),data=ref<View>(),enrichment=ref<Enrichment>(),proposal=ref<Proposal>();
const organization=ref(''),includeIsolated=ref(true),selectedFamilies=ref<string[]>([]),scopeConsent=ref(false);
const collectFirst=ref(false),collectionTargets=ref(''),allowHunter=ref(false);
const busy=ref(false),reading=ref(false),error=ref(''),readError=ref(''),notice=ref(''),showConflicts=ref(false),showEvidence=ref(false),showCosts=ref(false);
const filter=ref('all'),search=ref(''),page=ref(1);
let boundRun='',boundPipeline='unbound',generation=0,sequence=0,evidenceSequence=0,disposed=false,timer:ReturnType<typeof setTimeout>|undefined,searchTimer:ReturnType<typeof setTimeout>|undefined;
const run=computed(()=>assist.value?.review?.run);
const phase=computed(()=>['blocked','paused'].includes(assist.value?.pipeline?.phase||'')?assist.value!.pipeline!.phase:assist.value?.phase||'idle');
const pipeline=computed(()=>assist.value?.pipeline);
const pipelineWorking=computed(()=>assist.value?.enabled===true&&!!pipeline.value&&!['awaiting_scope','steady','blocked','paused'].includes(pipeline.value.phase));
const modelRetry=computed(()=>phase.value==='blocked'&&!needsInitialCollection.value&&enrichment.value?.run?.status!=='waiting_storage');
const analyzing=computed(()=>['preparing','analyzing','pausing'].includes(phase.value)&&assist.value?.enabled===true);
const supplementing=computed(()=>enrichment.value?.run?.status==='pausing'||!!enrichment.value?.run?.enabled&&['running','waiting','waiting_storage'].includes(enrichment.value.run.status));
const working=computed(()=>analyzing.value||supplementing.value||pipelineWorking.value);
const targetChanged=computed(()=>{const saved=assist.value?.organization||run.value?.organization;return !!saved&&organization.value.trim()!==saved;});
const proposalDiagnostics=computed(()=>!targetChanged.value?proposal.value:undefined);
const visibleProposal=computed(()=>!targetChanged.value&&!['paused','blocked'].includes(phase.value)?proposal.value:undefined);
function hasBoundEvidence(group:ScopeGroup){return group.verification?.status==='bound_local_evidence'&&group.verification.sources?.some(s=>!!s.kind?.trim()&&!!s.host?.trim());}
const eligibleGroups=computed(()=>(visibleProposal.value?.groups||[]).filter(hasBoundEvidence));
const heldFamilies=computed(()=>Math.max(proposalDiagnostics.value?.heldForVerification||0,(proposalDiagnostics.value?.groups||[]).filter(g=>!hasBoundEvidence(g)).length));
const selectedHosts=computed(()=>[...new Set(eligibleGroups.value.filter(g=>selectedFamilies.value.includes(g.root)).flatMap(g=>g.scopes))]);
function sourceLabel(kind:string){return ({tls_subject:'TLS 证书 Subject',icp:'备案单位',confirmed_directory:'已确认单位目录',public_ct_subject:'公开 CT 叶证书 Subject'} as Record<string,string>)[kind]||kind;}
const certificateConflicts=computed(()=>enrichment.value?.conflicts?.filter(c=>c.decision==='pending')||[]);
const conflictCount=computed(()=>(assist.value?.review?.run?.queueCounts?.conflict||0)+certificateConflicts.value.length);
const conflicts=computed(()=>(assist.value?.review?.items||[]).filter(i=>i.decision==='pending'&&i.state==='done'&&i.advice==='conflict'));
const needsCollection=computed(()=>assist.value?.hasLocalAssets===false);
const needsInitialCollection=computed(()=>needsCollection.value&&!pipeline.value);
const existingRoots=computed(()=>enrichment.value?.roots||assist.value?.review?.roots||[]);
const pages=computed(()=>Math.max(1,Math.ceil((data.value?.total||0)/30)));
const progress=computed(()=>!run.value?.totalAssets?0:Math.min(100,Math.round(run.value.processedAssets/run.value.totalAssets*100)));
const summary=computed(()=>{
 if(pipelineWorking.value)return pipeline.value?.reason||'后台正在自动接续发现流程，你可以离开此页面。';
 if(needsInitialCollection.value)return '先用 FOFA 采集初始资产，再自动核验、提炼种子并继续发现；无需切换页面。';
 if(targetChanged.value)return '目标已修改。点击开始后，AI 会按新目标重新整理，原记录保留。';
 if(phase.value==='pausing'||enrichment.value?.run?.status==='pausing')return '正在保存进度并暂停，不会领取新的工作。';
 if(phase.value==='preparing')return '正在整理整个项目的资产和证书，你可以离开此页面。';
 if(phase.value==='analyzing')return 'AI 正在判断相关性、合并同类线索，完成后自动汇总范围。';
 if(eligibleGroups.value.length)return '通过本地证据核验的范围已汇总。统一确认后，AI 会继续自动补证。';
 if(heldFamilies.value)return '缺少逐域绑定依据的建议已暂存等待核验，不会作为新增范围要求你逐项确认。';
 if(phase.value==='blocked')return (assist.value?.reason||'模型分析暂时无法继续，已完成的结果保留。')+(supplementing.value?' 已确认范围的免费补证仍在进行，可单独重试 AI 分析。':'');
 if(phase.value==='paused')return '已暂停，继续时会接着处理，不重新发送已完成的模型请求。';
 if(supplementing.value)return '正在已确认范围内自动补证和持续发现，不需要逐条操作。';
 if(phase.value==='ready')return '本轮整理已完成。相关结果保留，证据不足的线索继续暂存。';
 return '告诉 AI 要查找的单位，整理、识别和补证交给它完成。';
});
const phaseLabel=computed(()=>phase.value==='pausing'||enrichment.value?.run?.status==='pausing'?'正在保存进度':pipelineWorking.value?'自动发现中':needsInitialCollection.value?'先采集初始资产':analyzing.value?'AI 工作中':eligibleGroups.value.length?'等待范围确认':phase.value==='blocked'?'需要处理':phase.value==='paused'?'已暂停':supplementing.value?'自动发现中':phase.value==='ready'?'本轮已完成':'准备开始');
const mainLabel=computed(()=>needsInitialCollection.value||collectFirst.value?'开始采集与 AI 发现':targetChanged.value?'按新目标开始':phase.value==='paused'?'从断点继续':run.value||assist.value?.hasLocalAssets?'继续增量发现':'开始 AI 发现');
const reviewWaiting=computed(()=>(run.value?.queueCounts?.pending||0)+(run.value?.queueCounts?.analyzing||0));
const reviewHandled=computed(()=>Math.max(0,(run.value?.groups||0)-reviewWaiting.value));
const counts=computed(()=>enrichment.value?.run?.counts||{});
const labels:Record<string,string>={retain:'AI 建议相关',conflict:'证据存在冲突',evidence:'证据不足，系统暂存',deferred:'暂存，等待新的关联证据',exclude:'AI 建议无关',resolved:'已按现有范围或规则处理',unknown:'请求结果未知，未自动重发',pending:'等待 AI 分析',analyzing:'AI 分析中'};
function setProposal(value:Proposal|null|undefined){
 if((value?.token||'')!==(proposal.value?.token||'')){
  const previous=proposal.value,selected=new Set(selectedFamilies.value),known=new Set(previous?.groups.map(g=>g.root)||[]);
  proposal.value=value||undefined;
  selectedFamilies.value=value?.groups.filter(g=>hasBoundEvidence(g)&&(!known.has(g.root)||selected.has(g.root))).map(g=>g.root)||[];
  scopeConsent.value=false;
  if(previous&&value&&!busy.value)notice.value='AI 更新了相关证据，已保留你取消的范围；请核对更新后再统一确认。';
 }
}
function schedule(g=generation){clearTimeout(timer);if(!disposed&&g===generation&&props.projectId)timer=setTimeout(()=>void load(g),working.value?1800:8000);}
async function load(g=generation){
 const projectId=props.projectId,seq=++sequence;if(!projectId||disposed||g!==generation)return;reading.value=true;
 const results=await Promise.allSettled([invoke<Assist>('seed_review_assist_status',{projectId}),invoke<Enrichment>('enrichment_action',{projectId,action:'status',args:{}})]);
 if(disposed||g!==generation||seq!==sequence)return;
 if(results[0].status==='fulfilled'){
  readError.value='';const value=results[0].value;assist.value=value;if(!showEvidence.value)data.value=value.review;
  if(value.review?.run&&value.review.run.id!==boundRun){boundRun=value.review.run.id;includeIsolated.value=value.review.run.includeIsolated??true;}
  if(!organization.value.trim())organization.value=value.organization||value.review?.run?.organization||'';
  if(value.pipeline){const binding=`${value.pipeline.collectionRunId??''}:${value.runId??value.review?.run?.id??''}`;if(binding!==boundPipeline){boundPipeline=binding;allowHunter.value=value.pipeline.allowHunter;collectFirst.value=false;collectionTargets.value='';}}
  else if(value.hasLocalAssets===false)collectFirst.value=true;
  setProposal(value.proposal);
 }else readError.value=`读取 AI 任务失败：${String(results[0].reason)}`;
 if(results[1].status==='fulfilled')enrichment.value=results[1].value;
 else if(results[0].status==='fulfilled')readError.value='自动补证进度暂时无法读取，已保留当前结果，系统会重试。';
 reading.value=false;schedule(g);
}
async function control(action:'start'|'pause'){
 const projectId=props.projectId,g=generation;if(!projectId||busy.value||action==='start'&&!organization.value.trim())return;
 if(action==='start'&&pipeline.value?.hunter?.costUnknown){error.value='Hunter 请求或扣费结果未知，已保留回执等待核对；不会重新发送该请求。';return;}
 busy.value=true;error.value='';notice.value='';
 try{
  const firstCollection=needsInitialCollection.value||collectFirst.value;
  const value=await invoke<Assist>('control_seed_assist',{input:{projectId,action,organization:organization.value.trim(),includeIsolated:includeIsolated.value,...(action==='start'?{collectFirst:firstCollection,...(firstCollection?{collectionTargets:collectionTargets.value.split(/\r?\n/).map(line=>line.trim()).filter(line=>line&&!line.startsWith('#'))}:{}),allowHunter:allowHunter.value}:{})}});
  if(disposed||g!==generation)return;assist.value=value;setProposal(value.proposal);await load(g);
  if(g===generation)notice.value=action==='pause'?'已请求暂停，完成的结果和断点会保留。':'后台已接手采集、核验和发现，完成当前阶段后自动继续。';
 }catch(e){if(g===generation)error.value=String(e);}finally{if(g===generation)busy.value=false;}
}
async function confirmScope(){
 const projectId=props.projectId,g=generation,p=visibleProposal.value;if(!projectId||!run.value||!p||busy.value||!scopeConsent.value||!selectedHosts.value.length)return;
 busy.value=true;error.value='';notice.value='';
 try{
  const result=await invoke<{enrichmentStarted:boolean;enrichmentError?:string;triageError?:string}>('confirm_seed_review_proposal',{input:{projectId,runId:run.value.id,hosts:[...selectedHosts.value],expectedRoots:p.roots,proposalToken:p.token,itemKey:null,userConfirmed:true}});
  if(disposed||g!==generation)return;setProposal(null);await load(g);
  notice.value=result.enrichmentStarted?'范围已确认，AI 正在自动补证。你可以直接查看结果。':`范围已保存，补证暂未启动：${result.enrichmentError||'请稍后点击继续'}。无需重复确认范围。`;
  if(result.triageError)notice.value+=` 证据关联待重试：${result.triageError}`;
  emit('confirmed');
 }catch(e){if(g===generation){error.value=String(e);await load(g);}}finally{if(g===generation)busy.value=false;}
}
async function decide(item:Item,decision:string){
 const projectId=props.projectId,g=generation;if(!projectId||busy.value||!run.value)return;busy.value=true;error.value='';
 try{await invoke('decide_seed_review',{input:{projectId,runId:run.value.id,key:item.key,expectedRevision:item.revision,decision}});if(disposed||g!==generation)return;await load(g);notice.value='决定已保存，AI 会沿用这次判断；探测范围仍需统一确认。';}
 catch(e){if(g===generation)error.value=String(e);}finally{if(g===generation)busy.value=false;}
}
async function acknowledgeCertificate(item:EnrichmentConflict){
 const projectId=props.projectId,g=generation;if(!projectId||busy.value)return;busy.value=true;error.value='';
 try{const value=await invoke<Enrichment>('enrichment_action',{projectId,action:'review_conflict',args:{kind:item.kind,identity:item.identity,expectedRevision:item.revision,decision:'acknowledged'}});if(disposed||g!==generation)return;enrichment.value=value;notice.value='冲突证据已保留，AI 不会据此扩大范围或调用付费查询。';}
 catch(e){if(g===generation)error.value=String(e);}finally{if(g===generation)busy.value=false;}
}
async function loadEvidence(reset=false){
 const projectId=props.projectId,g=generation,seq=++evidenceSequence;if(!projectId||!showEvidence.value)return;if(reset)page.value=1;
 try{const value=await invoke<View>('seed_review_status',{projectId,runId:run.value?.id,filter:filter.value,page:page.value,search:search.value.trim()});if(!disposed&&g===generation&&seq===evidenceSequence)data.value=value;}
 catch(e){if(g===generation&&seq===evidenceSequence)error.value=String(e);}
}
function query(){clearTimeout(searchTimer);searchTimer=setTimeout(()=>void loadEvidence(true),250);}
watch(selectedFamilies,()=>{scopeConsent.value=false;},{deep:true});
watch(organization,()=>{scopeConsent.value=false;if(targetChanged.value)allowHunter.value=false;});
watch(filter,()=>void loadEvidence(true));
watch(showEvidence,value=>{if(value)void loadEvidence(true);});
watch(()=>props.projectId,()=>{
 generation++;sequence++;evidenceSequence++;clearTimeout(timer);clearTimeout(searchTimer);
 boundRun='';boundPipeline='unbound';assist.value=undefined;data.value=undefined;enrichment.value=undefined;proposal.value=undefined;organization.value='';selectedFamilies.value=[];scopeConsent.value=false;collectFirst.value=false;collectionTargets.value='';allowHunter.value=false;
 busy.value=false;reading.value=false;error.value='';readError.value='';notice.value='';showConflicts.value=false;showEvidence.value=false;showCosts.value=false;page.value=1;search.value='';
 if(props.projectId)void load();
},{immediate:true});
watch(()=>props.refreshVersion,()=>{if(!busy.value)void load();});
onUnmounted(()=>{disposed=true;generation++;clearTimeout(timer);clearTimeout(searchTimer);});
</script>
<template>
<section class="ai-discovery" data-testid="seed-review-workbench">
 <header class="ai-discovery-heading"><div><h2><Bot :size="21"/>AI 发现资产</h2><p>FOFA 首采 → AI 核验 → 免费发现 → Hunter 精确补查 → 去重回流。新增范围集中确认，真正的冲突再交给你。</p></div><span class="phase" :class="{active:working,attention:phase==='blocked'||!!eligibleGroups.length}" role="status">{{phaseLabel}}</span></header>
 <p v-if="!projectId" class="empty-state">先选择或新建一个工作空间。</p>
 <template v-else>
  <div class="ai-target"><label class="field"><span>要查找的单位或业务</span><input v-model="organization" maxlength="240" placeholder="例如：中国移动集团及其下属单位" :disabled="busy||working"/></label>
   <button v-if="(!working||modelRetry)&&(!eligibleGroups.length||targetChanged)" class="button primary" :disabled="busy||reading||!organization.trim()||pipeline?.hunter.costUnknown" data-ai-action="start" @click="control('start')"><Play :size="16"/>{{pipeline?.hunter.costUnknown?'等待查询回执核对':busy?'正在接续…':modelRetry?'重试并继续':mainLabel}}</button>
   <button v-if="working" class="button ghost" :disabled="busy||phase==='pausing'||enrichment?.run?.status==='pausing'" data-ai-action="pause" @click="control('pause')"><Pause :size="15"/>暂停</button>
  </div>
  <div v-if="!working&&!eligibleGroups.length" class="discovery-start-options">
   <label v-if="!needsInitialCollection" class="discovery-option"><input v-model="collectFirst" type="checkbox" :disabled="busy"/><span>{{pipeline?'重新采集 FOFA，再开始新一轮发现':'先增量采集 FOFA，再接续 AI 发现'}}<small>{{pipeline?'不勾选则接续已保存的任务与断点，不重复首采。':'不勾选则复用本项目已有资产。'}}</small></span></label>
   <label v-if="needsInitialCollection||collectFirst" class="field collection-targets"><span>FOFA 初始种子（可选，每行一个单位、域名、IP 或网段）</span><textarea v-model="collectionTargets" rows="2" :disabled="busy" placeholder="例如：10086.cn&#10;不填写时按上方目标单位被动搜索"/><small>首采只查询测绘数据，不把检索目标自动当作主动探测授权。</small></label>
   <label class="discovery-option hunter-consent"><input v-model="allowHunter" type="checkbox" :disabled="busy" data-testid="discovery-allow-hunter"/><span>本次启用 Hunter 精确关联<small>仅使用已核验且允许查询的证书指纹、域名或 CNAME，自动翻页并去重。可使用全部免费与权益积分，展示实际消耗；结果或扣费未知时暂停，不自动重发。</small></span></label>
  </div>
  <p class="ai-next-step" role="status">{{summary}}</p>
  <p v-if="!run&&!needsCollection" class="cost-hint">使用当前聊天模型分析，显示模型用量；Hunter 需勾选后随本次运行启用。</p>
  <DiscoveryPipelineProgress v-if="pipeline" :pipeline="pipeline"/>
  <section v-if="run" class="ai-work-status" aria-label="AI 工作进度">
   <div class="status-line"><strong>{{analyzing?'AI 正在处理整个项目':'当前项目'}} · {{run.processedAssets.toLocaleString()}} / {{run.totalAssets.toLocaleString()}} 条资产</strong><button v-refresh-feedback="{scope:projectId,busy:reading,error:error||readError}" class="text-button" :disabled="reading||busy" aria-label="刷新 AI 进度" @click="load()"><RefreshCw :size="14"/></button></div>
   <progress v-if="phase==='preparing'" :value="progress" max="100" aria-label="本地整理进度"/>
   <p v-if="run.groups" class="review-progress">已自动处理 {{reviewHandled.toLocaleString()}} / {{run.groups.toLocaleString()}} 组证据<span v-if="reviewWaiting"> · 待分析 {{reviewWaiting.toLocaleString()}} 组</span><span v-if="analyzing"> · 正在继续</span></p><progress v-if="analyzing&&phase!=='preparing'" :value="reviewHandled" :max="run.groups||1" aria-label="AI 与规则处理进度"/>
   <div class="ai-status-metrics"><span>已整理证据 <b>{{run.groups.toLocaleString()}}</b></span><span>已核验范围建议 <b>{{eligibleGroups.length}}</b></span><button v-if="conflictCount" class="conflict-action" :aria-expanded="showConflicts" @click="showConflicts=!showConflicts">需要你决定 <b>{{conflictCount}}</b><ArrowRight :size="13"/></button><span v-else>需要你决定 <b>0</b></span></div>
   <p v-if="enrichment?.run" class="enrichment-summary">自动补证：已完成 {{counts.done||0}} 项<span v-if="(counts.pending||0)+(counts.running||0)">，正在处理 {{(counts.pending||0)+(counts.running||0)}} 项</span><span v-if="counts.retry">，{{counts.retry}} 项未成功，{{enrichment.run.enabled?'系统保留并重试':'记录已保留'}}</span><span v-if="counts.failed">，{{counts.failed}} 项永久错误或已达到重试上限，详情已保留</span>。<template v-if="!pipeline">Hunter 本轮 {{enrichment.hunterRequests||0}} 次请求。</template></p>
   <p v-if="existingRoots.length" class="confirmed-scope">已允许自动处理：{{existingRoots.slice(0,3).join('、')}}{{existingRoots.length>3?` 等 ${existingRoots.length} 个范围`:''}}</p>
   <button class="button ghost compact" data-ai-action="results" @click="emit('results')">查看资产结果<ArrowRight :size="14"/></button>
  </section>
  <section v-if="visibleProposal&&eligibleGroups.length" class="ai-scope-confirm" aria-label="统一确认发现范围">
   <h3>已核验的范围建议</h3><p>共 {{eligibleGroups.length}} 个域名家族，已核对本地证据与目标单位的绑定；共享平台已限定具体主机。</p>
   <p v-if="visibleProposal.removedNoLiveEvidence">已省略 {{visibleProposal.removedNoLiveEvidence}} 项只有历史或已失效证据的范围，原记录保留。</p>
   <details class="scope-adjust"><summary>查看或调整建议 · 已选 {{selectedFamilies.length}} / {{eligibleGroups.length}} 组</summary><div class="scope-family-list"><article v-for="group in eligibleGroups" :key="group.root"><label class="scope-family-choice"><input v-model="selectedFamilies" type="checkbox" :value="group.root" :disabled="busy"/><span><strong>{{group.root}}</strong><small>{{group.assetCount.toLocaleString()}} 条关联资产 · {{group.evidenceGroups}} 组证据{{group.shared?` · 共享平台，仅 ${group.scopes.length} 个具体主机`:''}}</small></span></label><div class="scope-verification"><p v-for="(source,index) in group.verification?.sources" :key="index">{{sourceLabel(source.kind)}} · {{source.host}} · {{source.organization||'已确认的域名绑定'}}<template v-if="source.assetId"> · 资产 #{{source.assetId}}</template><small v-if="source.validUntil"> · 证书有效至 {{source.validUntil}}</small><small v-if="source.sourceUrl" class="verification-source-url">{{source.sourceUrl}}</small></p></div><details class="scope-boundary"><summary>范围与证据</summary><p>仅确认以下范围（及其子域）：{{group.scopes.join('、')}}</p><p>已观察名称示例：{{group.observedDomains.join('、')}}</p><p v-if="group.reasons.length">AI 推断（不作核验依据）：{{group.reasons.join('；')}}</p><p v-if="group.conflictGroups">{{group.conflictGroups}} 组排除或身份冲突保留原结论，不会据此恢复资产。</p></details></article></div></details>
   <label class="scope-consent"><input v-model="scopeConsent" type="checkbox" :disabled="busy"/>确认所选域名及子域属于本次允许探测的范围。</label>
   <button class="button primary" :disabled="busy||!scopeConsent||!selectedHosts.length||selectedHosts.length>500" data-ai-action="confirm" @click="confirmScope"><Check :size="16"/>{{busy?'正在接续…':'确认范围，让 AI 自动继续'}}</button>
   <p v-if="selectedHosts.length>500" class="form-error">建议范围超过当前项目保存上限，请按更具体的业务单位分析，或在建议中取消部分域名家族。</p>
   <p class="scope-note">确认后自动补证，不恢复隔离或回收资产。{{pipeline?.allowHunter?'随后按本次已保存的 Hunter 授权进行精确关联，费用展示在流程进度中。':'Hunter 未启用，不会发送付费查询。'}}</p>
  </section>
  <p v-if="proposalDiagnostics" class="cost-hint">{{proposalDiagnostics.networkVerified?'本次包含公开 CT 叶证书核验；证书证据不代表主机当前存活，新增探测范围仍需确认。':'本次仅核验本地证据，未联网查证企业归属。'}}</p>
  <p v-if="heldFamilies" class="scope-held" role="status">{{heldFamilies}} 个域名家族缺少逐域绑定依据，已暂存等待核验，不加入探测范围，无需逐项确认。<span v-if="proposalDiagnostics?.removedUnverified">本次拦截 {{proposalDiagnostics.removedUnverified}} 项未核验范围。</span></p>
  <section v-if="showConflicts&&conflictCount" class="ai-conflicts" aria-label="需要人工决定的冲突">
   <h3>这些证据存在冲突，需要你决定</h3><p>其他证据不足和明确无关的线索由系统暂存或分流，不需要逐条点击。</p>
   <article v-for="item in conflicts" :key="item.key"><strong>{{item.evidence.domains.slice(0,4).join('、')||'证书身份冲突'}}</strong><p>{{item.queueReason||item.reason}}</p><details><summary>查看依据</summary><p>证书单位：{{item.evidence.subjectOrganizations.join('、')||'未取得'}}</p><p>单位线索：{{[...(item.evidence.organizationHints||[]),...(item.evidence.icpOrganizations||[])].join('、')||'未取得'}}</p><p>{{item.assetCount}} 条资产；示例 #{{item.evidence.exampleAssetIds.join('、')}}</p></details><div class="conflict-actions"><button class="button ghost compact" :disabled="busy" @click="decide(item,'accepted')">确认相关</button><button class="button ghost compact" :disabled="busy" @click="decide(item,'rejected')">排除这组线索</button></div></article>
   <article v-for="item in certificateConflicts" :key="item.kind+item.identity"><strong>证书与主机身份不一致</strong><p>{{item.hosts.join('、')}}{{item.hostCount&&item.hostCount>item.hosts.length?` 等 ${item.hostCount} 个主机`:''}}</p><p>{{item.detail}}</p><details><summary>证书标识</summary><p>{{item.identity}}</p></details><button class="button ghost compact" :disabled="busy" @click="acknowledgeCertificate(item)">我已了解，保留冲突记录</button></article>
   <p v-if="(assist?.review.run?.queueCounts?.conflict||0)>conflicts.length">更多相关性冲突可在完整证据记录中查看。</p>
  </section>
  <p v-if="notice" class="ai-notice" role="status">{{notice}}</p><p v-if="error||readError||assist?.enrichmentError" class="form-error" role="alert">{{error||readError||assist?.enrichmentError}}</p>
  <details class="ai-details" :open="showCosts" @toggle="showCosts=($event.target as HTMLDetailsElement).open"><summary>模型用量与处理说明</summary><p>AI 使用当前模型，可能产生模型费用。Hunter 仅按本次运行保存的显式授权启用，显示每页及累计实际积分。</p><p v-if="run">此审阅任务累计：{{run.requests}} 次请求，输入 {{run.inputTokens.toLocaleString()}} / 输出 {{run.outputTokens.toLocaleString()}} Token{{run.unknownRequests?`；${run.unknownRequests} 次结果未知，不自动重发`:''}}。{{run.model}}</p><p>证据不足的外部线索继续暂存，等待新的关联证据，不是人工待办。AI 建议不会自动变成新增探测授权。</p><label class="scope-consent"><input v-model="includeIsolated" type="checkbox" :disabled="busy||working"/>分析时包含隔离及回收记录，只读提取证据，不自动恢复资产。</label></details>
  <details v-if="run" class="ai-details" :open="showEvidence" @toggle="showEvidence=($event.target as HTMLDetailsElement).open"><summary>完整证据记录与诊断</summary><template v-if="showEvidence"><div class="evidence-filters"><select v-model="filter" aria-label="证据记录分类"><option value="all">全部记录</option><option value="conflict">需要决定的冲突</option><option value="retain">AI 建议相关</option><option value="evidence">系统暂存的缺证线索</option><option value="exclude">建议无关</option><option value="errors">请求异常</option><option value="accepted">人工认可</option><option value="rejected">人工排除</option></select><input v-model="search" placeholder="搜索域名、证书或单位" @input="query"/></div><div class="evidence-records"><article v-for="item in data?.items" :key="item.key"><strong>{{item.kind==='certificate'?item.target:item.evidence.domains.slice(0,5).join('、')}}</strong><span>{{item.decision==='accepted'?'人工认可':item.decision==='rejected'?'人工排除':labels[item.state==='done'?item.advice:item.state]||item.state}}</span><p>{{item.queueReason||item.reason}}</p><small>{{item.assetCount}} 条资产 · 来源 {{item.evidence.sources.join(' / ')}}</small></article></div><footer><span>{{data?.total.toLocaleString()}} 组 · {{page}} / {{pages}} 页</span><button class="button ghost compact" :disabled="page<=1" @click="page--;loadEvidence()"><ChevronLeft :size="13"/>上一页</button><button class="button ghost compact" :disabled="page>=pages" @click="page++;loadEvidence()">下一页<ChevronRight :size="13"/></button></footer></template></details>
 </template>
</section>
</template>
<style scoped>
.discovery-start-options{display:grid;gap:12px;margin:12px 0 16px;padding:14px;border:1px solid var(--line);border-radius:8px;background:var(--app-bg,#fafbfc)}.discovery-option{display:flex;align-items:flex-start;gap:9px;font-size:12px}.discovery-option input{width:16px;height:16px;flex:0 0 16px;margin:3px 0 0;accent-color:var(--blue)}.discovery-option span{min-width:0}.discovery-option small,.collection-targets small{display:block;color:var(--muted);font-size:11px;line-height:1.6;margin-top:3px}.collection-targets textarea{width:100%;box-sizing:border-box;min-height:70px;padding:9px 11px;border:1px solid var(--line);border-radius:7px;background:var(--app-surface,#fff);color:inherit;font:inherit;resize:vertical;line-height:1.6}.verification-source-url{display:block;overflow-wrap:anywhere}

.scope-verification{margin:5px 0 6px 25px;font-size:11px;color:var(--muted);overflow-wrap:anywhere}.scope-verification p{margin:3px 0}.scope-held{padding:10px 12px;border:1px solid var(--line);border-radius:7px;color:var(--muted);font-size:12px;overflow-wrap:anywhere}
.ai-discovery{max-width:1100px;margin:0 auto;padding:24px;border:1px solid var(--line,#e2e5e9);border-radius:12px;background:var(--app-surface,#fff);font-size:13px;line-height:1.65;min-width:0;box-sizing:border-box}.ai-discovery-heading{display:flex;justify-content:space-between;gap:16px;align-items:flex-start}.ai-discovery h2{display:flex;align-items:center;gap:9px;margin:0;font-size:20px}.ai-discovery h3{font-size:15px;margin:0 0 7px}.ai-discovery-heading p,.ai-discovery section>p,.ai-details p{color:var(--muted,#727984);margin:6px 0 12px}.phase{white-space:nowrap;font-size:11px;border:1px solid var(--line);border-radius:5px;padding:3px 8px;color:var(--muted)}.phase.active{color:#1b6d51;background:#edf8f2;border-color:#c7e9d8}.phase.attention{color:#285d9b;background:#eff5fd;border-color:#c6d9f4}.ai-target{display:flex;align-items:flex-end;gap:10px;margin:22px 0 10px}.field{display:flex;flex-direction:column;gap:7px;flex:1;min-width:0}.field>span{font-weight:550;font-size:12px}.ai-target input,.evidence-filters input,.evidence-filters select{box-sizing:border-box;border:1px solid var(--line);border-radius:7px;background:var(--app-surface,#fff);padding:9px 11px;color:inherit;font:inherit;min-height:40px;min-width:0}.ai-target .button{min-height:40px;flex-shrink:0}.ai-next-step{margin:6px 0 18px;color:var(--muted)}.cost-hint{font-size:11px;color:var(--muted)}.ai-work-status{padding:16px;border:1px solid var(--line);border-radius:8px;background:var(--app-bg,#fafbfc);margin:16px 0}.status-line{display:flex;justify-content:space-between;align-items:center;gap:12px}.status-line strong{font-size:12px}.ai-work-status progress{width:100%;height:6px;margin-top:10px;accent-color:var(--blue)}.ai-status-metrics{display:flex;flex-wrap:wrap;gap:10px 24px;margin:13px 0;color:var(--muted)}.ai-status-metrics b{color:var(--text,#30343b);margin-left:7px;font-size:15px}.conflict-action{display:flex;align-items:center;gap:6px;border:0;padding:0;background:transparent;color:#936013;font:inherit;cursor:pointer}.ai-work-status .enrichment-summary,.ai-work-status .confirmed-scope{font-size:11px;overflow-wrap:anywhere}.ai-scope-confirm{padding:18px;border:1px solid #b9d1ef;border-radius:9px;background:#f9fbff;margin:18px 0}.scope-adjust>summary{cursor:pointer;color:#37669b;padding:7px 0}.scope-family-list{max-height:360px;overflow:auto;overscroll-behavior:contain;margin:8px 0 16px;border-top:1px solid var(--line)}.scope-family-list article{padding:12px 2px;border-bottom:1px solid var(--line)}.scope-family-choice{display:flex;align-items:flex-start;gap:9px}.scope-family-choice span{min-width:0}.scope-family-choice strong{display:block;overflow-wrap:anywhere}.scope-family-choice small{display:block;color:var(--muted);font-size:11px}.scope-reason{margin:5px 0 3px 25px;color:var(--muted);font-size:11px}.scope-boundary{font-size:11px;margin-left:25px;color:var(--muted);overflow-wrap:anywhere}.scope-boundary summary{cursor:pointer}.scope-consent{display:flex;align-items:flex-start;gap:9px;margin:15px 0;font-size:12px}.scope-consent input,.scope-family-choice input{width:16px;height:16px;min-height:0;padding:0;flex:0 0 16px;margin:3px 0;accent-color:var(--blue)}.ai-scope-confirm .scope-note{font-size:11px;margin:10px 0 0}.ai-conflicts{padding:16px;border:1px solid #ead6b7;border-radius:8px;margin:18px 0}.ai-conflicts article{padding:12px 0;border-top:1px solid var(--line);overflow-wrap:anywhere}.ai-conflicts article>p{margin:5px 0;color:var(--muted)}.ai-conflicts details{font-size:11px;color:var(--muted)}.ai-conflicts summary{cursor:pointer}.conflict-actions{display:flex;gap:8px;margin-top:9px}.ai-notice{color:#246c4e;font-size:12px}.ai-details{border-top:1px solid var(--line);margin-top:18px;padding-top:12px;font-size:11px;color:var(--muted)}.ai-details>summary{cursor:pointer}.ai-details p{margin-top:10px}.evidence-filters{display:grid;grid-template-columns:210px minmax(0,1fr);gap:10px;margin-top:12px}.evidence-filters input,.evidence-filters select{font-size:12px;min-height:34px}.evidence-records{max-height:380px;overflow:auto;margin:12px 0}.evidence-records article{border-top:1px solid var(--line);padding:10px 0;overflow-wrap:anywhere}.evidence-records strong{display:block;color:var(--text);font-size:12px}.evidence-records span{display:block;margin-top:4px}.evidence-records p{margin:4px 0}.ai-details footer{display:flex;align-items:center;justify-content:flex-end;gap:7px}.ai-details footer>span{margin-right:auto}.button:disabled{cursor:default}@media(max-width:740px){.ai-discovery{padding:16px}.ai-discovery-heading{flex-wrap:wrap}.ai-target{flex-wrap:wrap}.ai-target .field{flex-basis:100%}.ai-target .button{flex:1}.evidence-filters{grid-template-columns:1fr}.ai-status-metrics{gap:8px 16px}.ai-scope-confirm{padding:14px}.ai-details footer{flex-wrap:wrap}}
</style>
