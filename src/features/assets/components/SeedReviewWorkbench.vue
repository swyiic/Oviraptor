<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import {computed,onUnmounted,ref,watch} from 'vue';
import {Bot,Play,Pause,RefreshCw,Check,ChevronLeft,ChevronRight,ArrowRight} from '@lucide/vue';
import {invoke} from '../uiPorts';
import {rememberDiscoveryPrimaryDraft,takeDiscoveryPrimaryDraft} from '../discoveryPrimaryDraft';
import type {DiscoveryPipeline,DiscoveryVerification} from '../../agentHost/discoveryTypes';
import DiscoveryPipelineProgress from './DiscoveryPipelineProgress.vue';
const props=defineProps<{projectId?:number;refreshVersion?:number;archived?:boolean}>();
const emit=defineEmits<{confirmed:[];collect:[];results:[];configure:[]}>();
type Evidence={domains:string[];companies:string[];subjectOrganizations:string[];organizationHints?:string[];icpOrganizations?:string[];sources:string[];exampleAssetIds:number[];quarantinedAssets:number;ambiguousIdentity:boolean;evidenceTruncated:boolean};
type Item={key:string;kind:string;target:string;assetCount:number;state:string;advice:string;confidence:number;reason:string;queueReason?:string;decision:string;revision:number;evidence:Evidence};
type ReviewRun={includeIsolated?:boolean;id:string;organization:string;status:string;totalAssets:number;processedAssets:number;cursor:number;maxId:number;groups:number;counts:Record<string,number>;queueCounts:Record<string,number>;manualCounts:Record<string,number>;model:string;requests:number;inputTokens:number;outputTokens:number;unknownRequests:number;reason:string};
type View={run:ReviewRun|null;items:Item[];total:number;roots?:string[]};
type ScopeGroup={root:string;shared:boolean;scopes:string[];observedDomains:string[];observedCount:number;evidenceGroups:number;certificateGroups:number;pendingEvidenceGroups:number;conflictGroups:number;assetCount:number;reasons:string[];verification?:DiscoveryVerification};
type Proposal={token:string;roots:string[];groups:ScopeGroup[];totalEvidenceGroups:number;removedNoLiveEvidence?:number;heldForVerification?:number;removedUnverified?:number;verificationPolicyVersion?:number;networkVerified?:boolean};
type Assist={phase:string;enabled:boolean;reason:string;runId?:string;organization?:string;hasLocalAssets:boolean;readOnly?:boolean;review:View;proposal:Proposal|null;enrichmentError?:string;pipeline?:DiscoveryPipeline|null;readiness?:{hunter:{keyConfigured:boolean;contractVerified:boolean}}};
type ScopeConfirmation={projectId:number;runId:string;token:string;roots:string[];hosts:string[]};
type EnrichmentConflict={kind:string;identity:string;hosts:string[];hostCount?:number;detail:string;decision:string;revision:number};
type Enrichment={roots:string[];conflicts?:EnrichmentConflict[];run:null|{id:string;status:string;enabled:boolean;counts:Record<string,number>;reason:string};hunterRequests?:number};
const assist=ref<Assist>(),data=ref<View>(),enrichment=ref<Enrichment>(),proposal=ref<Proposal>();
const organization=ref(''),includeIsolated=ref(true),selectedFamilies=ref<string[]>([]);
const collectFirst=ref(false),collectionTargets=ref(''),allowHunter=ref(false);
const busy=ref(false),reading=ref(false),error=ref(''),readError=ref(''),notice=ref(''),showConflicts=ref(false),showEvidence=ref(false);
const filter=ref('all'),search=ref(''),page=ref(1);
const editingTarget=ref(false),showSettings=ref(false),showProcess=ref(false),showScopeDetails=ref(false),evidenceReading=ref(false);
let targetHunterDraft=false,draftRestored=false,restoredHunterTarget='';
let boundRun='',boundPipeline='unbound',generation=0,sequence=0,evidenceSequence=0,disposed=false,timer:ReturnType<typeof setTimeout>|undefined,searchTimer:ReturnType<typeof setTimeout>|undefined;
const run=computed(()=>assist.value?.review?.run);
const readOnly=computed(()=>props.archived||assist.value?.readOnly===true);
const savedOrganization=computed(()=>assist.value?.organization||run.value?.organization||'');
const showTargetInput=computed(()=>editingTarget.value||!savedOrganization.value||targetChanged.value);
const hunterReady=computed(()=>!assist.value?.readiness||assist.value.readiness.hunter.keyConfigured&&assist.value.readiness.hunter.contractVerified);
const hunterSetupRequired=computed(()=>allowHunter.value&&!hunterReady.value);
const hunterReceiptUnknown=computed(()=>pipeline.value?.hunter?.costUnknown===true);
const freeContinuation=computed(()=>hunterReceiptUnknown.value||hunterSetupRequired.value);
const optionsEditable=computed(()=>!readOnly.value&&!busy.value&&!working.value&&!eligibleGroups.value.length);
const statusUncertain=computed(()=>!!readError.value||!assist.value);
const knownHunterCost=computed(()=>typeof pipeline.value?.hunter?.spentPoints==='number'&&Number.isFinite(pipeline.value.hunter.spentPoints));
const pipelineStage=computed(()=>({collecting:'FOFA 正在采集',local_analysis:'AI 正在核验',passive_evidence:'正在补充证书依据',free_discovery:'已确认范围内自动发现',hunter:'Hunter 正在精确关联',waiting_storage:'等待本地存储，自动重试',steady:'本轮发现已完成'} as Record<string,string>)[pipeline.value?.phase||'']||'后台正在接续');
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
const scopeConfirmations=computed<ScopeConfirmation[]>(()=>props.projectId&&run.value&&visibleProposal.value?[{projectId:props.projectId,runId:run.value.id,token:visibleProposal.value.token,roots:[...visibleProposal.value.roots],hosts:[...selectedHosts.value]}]:[]);
function sourceLabel(kind:string){return ({tls_subject:'TLS 证书 Subject',icp:'备案单位',confirmed_directory:'已确认单位目录',public_ct_subject:'公开 CT 叶证书 Subject'} as Record<string,string>)[kind]||kind;}
const certificateConflicts=computed(()=>enrichment.value?.conflicts?.filter(c=>c.decision==='pending')||[]);
const conflictCount=computed(()=>(assist.value?.review?.run?.queueCounts?.conflict||0)+certificateConflicts.value.length);
const conflicts=computed(()=>(assist.value?.review?.items||[]).filter(i=>i.decision==='pending'&&i.state==='done'&&i.advice==='conflict'));
const needsCollection=computed(()=>assist.value?.hasLocalAssets===false);
const needsInitialCollection=computed(()=>needsCollection.value&&!pipeline.value);
const existingRoots=computed(()=>enrichment.value?.roots||assist.value?.review?.roots||[]);
const pages=computed(()=>Math.max(1,Math.ceil((data.value?.total||0)/30)));
const summary=computed(()=>{
 if(readOnly.value)return '此项目已归档，可查看资产、种子与历史记录。';
 if(statusUncertain.value)return assist.value?'状态暂时无法更新，已保留上次结果。先重试读取，再继续操作。':'先读取本项目进度；读取成功后再开始发现。';
 if(targetChanged.value)return '目标已修改。开始后按新目标分析，原记录保留。';
 if(phase.value==='pausing'||enrichment.value?.run?.status==='pausing')return '正在保存进度并暂停，请稍候。';
 if(phase.value==='blocked')return (pipeline.value?.reason||assist.value?.reason||'当前步骤未完成，进度已保存。')+(freeContinuation.value?' 可先继续免费发现，Hunter 原回执保留。':supplementing.value?' 免费补证仍在继续，可单独重试 AI。':'');
 if(phase.value==='paused')return '已暂停。继续后沿用已有任务与逐项结果，不重发已完成的模型请求。';
 if(pipelineWorking.value)return `${pipelineStage.value}。${pipeline.value?.reason||'当前阶段完成后自动继续，你可以离开此页面。'}`;
 if(needsInitialCollection.value)return '新项目会先用 FOFA 采集，再自动核验与扩展发现。';
 if(phase.value==='preparing')return '正在整理项目资产和证书，完成后自动分析。';
 if(phase.value==='analyzing')return 'AI 正在提炼相关种子，只有新增范围和真实冲突需要你决定。';
 if(eligibleGroups.value.length)return '新增范围已汇总。确认一次后，AI 自动继续。';
 if(supplementing.value)return '正在已确认范围内自动补证和持续发现，不需要逐条操作。';
 if(phase.value==='ready'||phase.value==='steady'||pipeline.value?.phase==='steady')return '本轮已完成；继续时只处理新增或变化的证据。';
 if(heldFamilies.value)return '证据不足的线索由系统继续补证，无需逐项确认。';
 return '填写目标后开始，采集、核验和扩展由系统自动接续。';
});
const phaseLabel=computed(()=>readOnly.value?'已归档':statusUncertain.value?'状态待刷新':phase.value==='pausing'||enrichment.value?.run?.status==='pausing'?'正在保存进度':phase.value==='blocked'?'需要处理':phase.value==='paused'?'已暂停':pipelineWorking.value?'自动发现中':needsInitialCollection.value?'准备首次采集':analyzing.value?'AI 工作中':eligibleGroups.value.length?'确认新增范围':supplementing.value?'自动发现中':phase.value==='ready'||pipeline.value?.phase==='steady'?'本轮已完成':'准备开始');
const mainLabel=computed(()=>freeContinuation.value?'继续免费发现':modelRetry.value?'重试并继续':needsInitialCollection.value||collectFirst.value?'开始采集与发现':targetChanged.value?'按新目标开始':phase.value==='paused'?'从断点继续':run.value||assist.value?.hasLocalAssets?'继续增量发现':'开始发现');
const projectAssetCount=computed(()=>pipeline.value?.counts?.collectedAssets??run.value?.totalAssets??(assist.value?.hasLocalAssets===false?0:null));
const proposalFamilyCount=computed(()=>(proposalDiagnostics.value?.groups||[]).filter(hasBoundEvidence).length);
const showPrimaryStart=computed(()=>!readOnly.value&&!statusUncertain.value&&(!working.value||modelRetry.value)&&(!eligibleGroups.value.length||targetChanged.value));
const showPrimaryRead=computed(()=>!readOnly.value&&statusUncertain.value);
function beginTargetEdit(){if(readOnly.value||busy.value||working.value)return;targetHunterDraft=allowHunter.value;editingTarget.value=true;}
function cancelTargetEdit(){organization.value=savedOrganization.value;editingTarget.value=false;allowHunter.value=targetHunterDraft;}
function configureHunter(){
 if(!props.projectId||readOnly.value||busy.value)return;
 rememberDiscoveryPrimaryDraft(props.projectId,{organization:organization.value,includeIsolated:includeIsolated.value,collectFirst:collectFirst.value,collectionTargets:collectionTargets.value,allowHunter:allowHunter.value,editingTarget:editingTarget.value,showSettings:showSettings.value});
 emit('configure');
}
const reviewWaiting=computed(()=>(run.value?.queueCounts?.pending||0)+(run.value?.queueCounts?.analyzing||0));
const reviewHandled=computed(()=>Math.max(0,(run.value?.groups||0)-reviewWaiting.value));
const counts=computed(()=>enrichment.value?.run?.counts||{});
const labels:Record<string,string>={retain:'AI 建议相关',conflict:'证据存在冲突',evidence:'证据不足，系统暂存',deferred:'暂存，等待新的关联证据',exclude:'AI 建议无关',resolved:'已按现有范围或规则处理',unknown:'请求结果未知，未自动重发',pending:'等待 AI 分析',analyzing:'AI 分析中'};
function setProposal(value:Proposal|null|undefined){
 if((value?.token||'')!==(proposal.value?.token||'')){
  const previous=proposal.value,selected=new Set(selectedFamilies.value),known=new Set(previous?.groups.map(g=>g.root)||[]);
  proposal.value=value||undefined;
  selectedFamilies.value=value?.groups.filter(g=>hasBoundEvidence(g)&&(!known.has(g.root)||selected.has(g.root))).map(g=>g.root)||[];
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
  if(!draftRestored){draftRestored=true;const draft=takeDiscoveryPrimaryDraft(projectId);if(draft){restoredHunterTarget=draft.organization;organization.value=draft.organization;includeIsolated.value=draft.includeIsolated;collectFirst.value=draft.collectFirst;collectionTargets.value=draft.collectionTargets;allowHunter.value=draft.allowHunter;editingTarget.value=draft.editingTarget;showSettings.value=draft.showSettings;}}
  setProposal(value.proposal);
 }else readError.value=`读取 AI 任务失败：${String(results[0].reason)}`;
 if(results[1].status==='fulfilled')enrichment.value=results[1].value;
 else if(results[0].status==='fulfilled')readError.value='自动补证进度暂时无法读取，已保留当前结果，系统会重试。';
 reading.value=false;schedule(g);
}
async function control(action:'start'|'pause',freeOnly=false){
 const projectId=props.projectId,g=generation;if(!projectId||readOnly.value||busy.value||action==='start'&&(!organization.value.trim()||statusUncertain.value))return;
 const paid=freeOnly?false:allowHunter.value;
 if(action==='start'&&paid&&(!hunterReady.value||hunterReceiptUnknown.value)){error.value=hunterReceiptUnknown.value?'Hunter 原回执待核对；可选择继续免费发现。':'Hunter 尚未就绪，请先配置账户，或继续免费发现。';return;}
 busy.value=true;error.value='';notice.value='';if(action==='start'&&freeOnly)allowHunter.value=false;
 try{
  const firstCollection=needsInitialCollection.value||collectFirst.value;
  const value=await invoke<Assist>('control_seed_assist',{input:{projectId,action,organization:organization.value.trim(),includeIsolated:includeIsolated.value,...(action==='start'?{collectFirst:firstCollection,...(firstCollection?{collectionTargets:collectionTargets.value.split(/\r?\n/).map(line=>line.trim()).filter(line=>line&&!line.startsWith('#'))}:{}),allowHunter:paid}:{})}});
  if(disposed||g!==generation)return;assist.value=value;setProposal(value.proposal);if(action==='start')editingTarget.value=false;await load(g);
  if(g===generation)notice.value=action==='pause'?'已请求暂停，完成的结果和断点会保留。':'后台已接手采集、核验和发现，完成当前阶段后自动继续。';
 }catch(e){if(g===generation)error.value=String(e);}finally{if(g===generation)busy.value=false;}
}
async function confirmScope(userClicked=false,displayed=scopeConfirmations.value[0]){
 const projectId=props.projectId,g=generation,p=visibleProposal.value;if(!userClicked||!projectId||readOnly.value||!run.value||!p||busy.value||statusUncertain.value||!selectedHosts.value.length||selectedHosts.value.length>500||!displayed)return;
 if(displayed.projectId!==projectId||displayed.runId!==run.value.id||displayed.token!==p.token||JSON.stringify(displayed.roots)!==JSON.stringify(p.roots)||JSON.stringify(displayed.hosts)!==JSON.stringify(selectedHosts.value)){notice.value='建议或选择已更新，请按当前显示的范围确认。';return;}
 busy.value=true;error.value='';notice.value='';
 try{
  const result=await invoke<{enrichmentStarted:boolean;enrichmentError?:string;triageError?:string}>('confirm_seed_review_proposal',{input:{projectId,runId:displayed.runId,hosts:[...displayed.hosts],expectedRoots:[...displayed.roots],proposalToken:displayed.token,itemKey:null,userConfirmed:true}});
  if(disposed||g!==generation)return;setProposal(null);await load(g);
  notice.value=result.enrichmentStarted?'范围已确认，AI 正在自动补证。你可以直接查看结果。':`范围已保存，补证暂未启动：${result.enrichmentError||'请稍后点击继续'}。无需重复确认范围。`;
  if(result.triageError)notice.value+=` 证据关联待重试：${result.triageError}`;
  emit('confirmed');
 }catch(e){if(g===generation){error.value=String(e);await load(g);}}finally{if(g===generation)busy.value=false;}
}
async function decide(item:Item,decision:string){
 const projectId=props.projectId,g=generation;if(!projectId||readOnly.value||busy.value||statusUncertain.value||!run.value)return;busy.value=true;error.value='';
 try{await invoke('decide_seed_review',{input:{projectId,runId:run.value.id,key:item.key,expectedRevision:item.revision,decision}});if(disposed||g!==generation)return;await load(g);notice.value='决定已保存，AI 会沿用这次判断；探测范围仍需统一确认。';}
 catch(e){if(g===generation)error.value=String(e);}finally{if(g===generation)busy.value=false;}
}
async function acknowledgeCertificate(item:EnrichmentConflict){
 const projectId=props.projectId,g=generation;if(!projectId||readOnly.value||busy.value||statusUncertain.value)return;busy.value=true;error.value='';
 try{const value=await invoke<Enrichment>('enrichment_action',{projectId,action:'review_conflict',args:{kind:item.kind,identity:item.identity,expectedRevision:item.revision,decision:'acknowledged'}});if(disposed||g!==generation)return;enrichment.value=value;notice.value='冲突证据已保留，AI 不会据此扩大范围或调用付费查询。';}
 catch(e){if(g===generation)error.value=String(e);}finally{if(g===generation)busy.value=false;}
}
async function loadEvidence(reset=false,requestedPage=page.value){
 const projectId=props.projectId,g=generation,seq=++evidenceSequence;if(!projectId||!showEvidence.value)return;const oldPage=page.value;const nextPage=reset?1:requestedPage;evidenceReading.value=true;
 try{const value=await invoke<View>('seed_review_status',{projectId,runId:run.value?.id,filter:filter.value,page:nextPage,search:search.value.trim()});if(!disposed&&g===generation&&seq===evidenceSequence){data.value=value;page.value=nextPage;}}
 catch(e){if(g===generation&&seq===evidenceSequence){error.value=String(e);page.value=oldPage;}}finally{if(g===generation&&seq===evidenceSequence)evidenceReading.value=false;}
}
function query(){clearTimeout(searchTimer);searchTimer=setTimeout(()=>void loadEvidence(true),250);}
watch(organization,value=>{if(restoredHunterTarget===value){restoredHunterTarget='';return;}if(targetChanged.value)allowHunter.value=false;});
watch(filter,()=>void loadEvidence(true));
watch(showEvidence,value=>{if(value)void loadEvidence(true);});
watch(()=>props.projectId,()=>{
 generation++;sequence++;evidenceSequence++;clearTimeout(timer);clearTimeout(searchTimer);
 boundRun='';boundPipeline='unbound';draftRestored=false;restoredHunterTarget='';assist.value=undefined;data.value=undefined;enrichment.value=undefined;proposal.value=undefined;organization.value='';selectedFamilies.value=[];collectFirst.value=false;collectionTargets.value='';allowHunter.value=false;
 busy.value=false;reading.value=false;error.value='';readError.value='';notice.value='';showConflicts.value=false;showEvidence.value=false;editingTarget.value=false;showSettings.value=false;showProcess.value=false;showScopeDetails.value=false;evidenceReading.value=false;page.value=1;search.value='';
 if(props.projectId)void load();
},{immediate:true});
watch(()=>props.refreshVersion,()=>{if(!busy.value)void load();});
onUnmounted(()=>{disposed=true;generation++;clearTimeout(timer);clearTimeout(searchTimer);});
</script>
<template>
<section class="ai-discovery" data-testid="seed-review-workbench" :aria-busy="busy">
 <header class="ai-discovery-heading"><div><h2><Bot :size="21"/>AI 发现资产</h2><p>告诉 AI 查谁，采集、核验和扩展自动接续。</p></div><div class="heading-status"><span class="phase" :class="{active:working&&!readOnly,attention:!readOnly&&(phase==='blocked'||!!eligibleGroups.length)}" role="status">{{phaseLabel}}</span><button v-refresh-feedback="{scope:projectId,busy:reading,error:readError}" class="button ghost icon-button" :disabled="reading||busy" aria-label="刷新 AI 进度" data-ai-action="refresh-status" @click="load()"><RefreshCw :size="15"/></button></div></header>
 <p v-if="!projectId" class="empty-state">先选择或新建一个工作空间。</p>
 <template v-else>
  <div class="discovery-hero">
   <div class="ai-target">
    <label v-if="showTargetInput" class="field"><span>要查找的单位或业务</span><input v-model="organization" maxlength="240" placeholder="例如：中国移动集团及其下属单位" :disabled="readOnly||busy||working" @keydown.enter.prevent="showPrimaryStart&&organization.trim()&&control('start',freeContinuation)"/></label>
    <div v-else class="saved-target"><span>要查找的单位或业务</span><strong>{{savedOrganization}}</strong><button v-if="!readOnly" class="text-button" :disabled="busy||working" data-ai-action="edit-target" @click="beginTargetEdit">修改</button></div>
    <button v-if="editingTarget" class="text-button cancel-target" :disabled="busy||working" data-ai-action="cancel-target" @click="cancelTargetEdit">取消修改</button>
   </div>
   <div class="primary-actions">
    <button v-if="showPrimaryRead" v-refresh-feedback="{scope:projectId,busy:reading,error:readError}" class="button primary" :disabled="reading||busy" data-ai-action="retry-read" @click="load()"><RefreshCw :size="16"/>{{reading?'正在读取…':'重试读取'}}</button>
    <button v-else-if="showPrimaryStart" class="button primary" :disabled="busy||reading||!organization.trim()" data-ai-action="start" @click="control('start',freeContinuation)"><Play :size="16"/>{{busy?'正在接续…':mainLabel}}</button>
    <button v-if="working&&!readOnly" class="button ghost" :disabled="busy||phase==='pausing'||enrichment?.run?.status==='pausing'" data-ai-action="pause" @click="control('pause')"><Pause :size="15"/>暂停</button>
    <button v-if="run||pipeline||assist?.hasLocalAssets" class="button ghost" data-ai-action="results" @click="emit('results')">查看资产<ArrowRight :size="14"/></button>
   </div>
  </div>
  <p class="ai-next-step" role="status">{{summary}}</p>
  <div class="run-context" v-if="pipeline"><span>第 {{pipeline.round||1}} 轮</span><span v-if="pipeline.collectionRunId">FOFA 任务 #{{pipeline.collectionRunId}}</span><span v-if="existingRoots.length">已允许 {{existingRoots.slice(0,2).join('、')}}{{existingRoots.length>2?` 等 ${existingRoots.length} 个范围`:''}}</span></div>
  <dl v-if="assist" class="discovery-metrics" aria-label="发现概览"><div><dt title="项目中的历史记录，包含隔离与回收记录；不等于已确认资产数">项目记录</dt><dd>{{projectAssetCount==null?'—':projectAssetCount.toLocaleString()}}</dd></div><div><dt>本轮新增</dt><dd>{{pipeline?.counts?.roundAdded==null?'—':pipeline.counts.roundAdded.toLocaleString()}}</dd></div><div><dt>需要你决定</dt><dd>{{proposalFamilyCount+conflictCount}}<button v-if="conflictCount" class="text-button" :aria-expanded="showConflicts" data-ai-action="review-conflicts" @click="showConflicts=!showConflicts">查看冲突<ArrowRight :size="13"/></button></dd><small>{{proposalFamilyCount}} 组新增范围 · {{conflictCount}} 组冲突</small></div></dl>
  <p v-if="analyzing&&run" class="review-progress">已自动处理 {{reviewHandled.toLocaleString()}} / {{run.groups.toLocaleString()}} 组证据<span v-if="reviewWaiting"> · 待分析 {{reviewWaiting.toLocaleString()}} 组</span><progress :value="phase==='preparing'?run.processedAssets:reviewHandled" :max="phase==='preparing'?(run.totalAssets||1):(run.groups||1)" aria-label="AI 与规则处理进度"/></p>
  <p v-if="enrichment?.run" class="enrichment-summary">自动补证：已完成 {{counts.done||0}} 项<span v-if="counts.running"> · 执行 {{counts.running}} 项</span><span v-if="counts.pending"> · 排队 {{counts.pending}} 项</span><span v-if="counts.retry"> · {{counts.retry}} 项未成功，{{enrichment.run.enabled?'系统保留并重试':'记录已保留'}}</span><span v-if="counts.failed"> · {{counts.failed}} 项永久错误或已达到重试上限，详情已保留</span></p>
  <div class="hunter-summary" :class="{unsettled:hunterReceiptUnknown}">
   <label v-if="optionsEditable" class="hunter-option"><input v-model="allowHunter" type="checkbox" :disabled="!hunterReady&&!allowHunter||hunterReceiptUnknown" data-testid="discovery-allow-hunter"/><span>使用 Hunter 精确扩展<small>{{allowHunter?'仅查询核验通过的种子，自动翻页与去重；按实际积分计费。':'先自动查 CT、DNS 和 TLS；Hunter 保持关闭。'}}</small></span></label>
   <span v-else class="hunter-mode">{{pipeline?.allowHunter?'Hunter 精确扩展已启用':'Hunter 未启用'}}</span>
   <span v-if="knownHunterCost" class="hunter-cost">已知消耗 {{pipeline!.hunter.spentPoints!.toLocaleString()}} 积分</span><span v-else-if="pipeline?.allowHunter" class="hunter-cost">积分回执尚未返回</span>
   <span v-if="hunterReceiptUnknown" class="hunter-receipt">部分扣费或请求结果未知，Hunter 已暂停；可继续免费发现，原回执保留。</span>
   <span v-if="!hunterReady" class="hunter-setup">Hunter 尚未就绪：{{!assist?.readiness?.hunter.keyConfigured?'需要保存 Key':'需要确认接口配置'}}。<button v-if="!readOnly" class="text-button" data-ai-action="configure" @click="configureHunter">前往配置</button></span>
  </div>
  <section v-if="visibleProposal&&eligibleGroups.length" class="ai-scope-confirm" aria-label="统一确认发现范围">
   <div class="scope-heading"><div><h3>{{readOnly?'历史范围建议':'只需确认新增范围'}}</h3><p>已核验的范围建议 · {{eligibleGroups.length}} 个域名家族；同一家族的相关证书一起处理。</p></div><span class="scope-count">已选 {{selectedHosts.length}} 个具体范围</span></div>
   <div class="scope-preview"><span v-for="group in eligibleGroups.slice(0,3)" :key="group.root">{{group.root}}<small v-if="group.shared">仅 {{group.scopes.length}} 个具体主机</small></span><span v-if="eligibleGroups.length>3">另 {{eligibleGroups.length-3}} 组</span></div>
   <details class="scope-adjust" :open="showScopeDetails" @toggle="showScopeDetails=($event.target as HTMLDetailsElement).open"><summary>查看或调整建议 · 已选 {{selectedFamilies.length}} / {{eligibleGroups.length}} 组</summary><div v-if="showScopeDetails" class="scope-family-list"><article v-for="group in eligibleGroups" :key="group.root"><label class="scope-family-choice"><input v-model="selectedFamilies" type="checkbox" :value="group.root" :disabled="busy||readOnly||statusUncertain"/><span><strong>{{group.root}}</strong><small>{{group.assetCount.toLocaleString()}} 条关联资产 · {{group.evidenceGroups}} 组证据{{group.shared?` · 共享平台，仅 ${group.scopes.length} 个具体主机`:''}}</small></span></label><div class="scope-verification"><p v-for="(source,index) in group.verification?.sources" :key="index">{{sourceLabel(source.kind)}} · {{source.host}} · {{source.organization||'已确认的域名绑定'}}<template v-if="source.assetId"> · 资产 #{{source.assetId}}</template><small v-if="source.validUntil"> · 证书有效至 {{source.validUntil}}</small><small v-if="source.sourceUrl" class="verification-source-url">{{source.sourceUrl}}</small></p></div><details class="scope-boundary"><summary>范围与证据</summary><p>仅确认以下范围（及其子域）：{{group.scopes.join('、')}}</p><p>已观察名称示例：{{group.observedDomains.join('、')}}</p><p v-if="group.reasons.length">AI 推断（不作核验依据）：{{group.reasons.join('；')}}</p><p v-if="group.conflictGroups">{{group.conflictGroups}} 组排除或身份冲突保留原结论，不会据此恢复资产。</p></details></article></div></details>
   <p v-if="!readOnly" class="scope-authorization">点击确认，表示所选 {{selectedHosts.length}} 个范围及其子域属于本次允许探测的范围。</p>
   <template v-if="!readOnly"><button v-for="confirmation in scopeConfirmations" :key="confirmation.token" class="button primary" :disabled="busy||reading||statusUncertain||!selectedHosts.length||selectedHosts.length>500" data-ai-action="confirm" @click="confirmScope(true,confirmation)"><Check :size="16"/>{{busy?'正在接续…':`确认这 ${selectedHosts.length} 个范围并继续`}}</button></template>
   <p v-if="selectedHosts.length>500" class="form-error">建议范围超过保存上限，请在上方取消部分域名家族，分批确认。</p>
   <p class="scope-note">{{pipeline?.allowHunter?'确认后自动补证，再按已保存的 Hunter 授权精确关联。':'确认后自动补证；Hunter 保持关闭。'}}隔离及回收记录不会自动恢复。</p>
  </section>
  <section v-if="showConflicts&&conflictCount" class="ai-conflicts" aria-label="需要人工决定的冲突">
   <h3>这些证据存在冲突，需要你决定</h3><p>其他证据不足和明确无关的线索由系统暂存或分流，不需要逐条点击。</p>
   <article v-for="item in conflicts" :key="item.key"><strong>{{item.evidence.domains.slice(0,4).join('、')||'证书身份冲突'}}</strong><p>{{item.queueReason||item.reason}}</p><details><summary>查看依据</summary><p>证书单位：{{item.evidence.subjectOrganizations.join('、')||'未取得'}}</p><p>单位线索：{{[...(item.evidence.organizationHints||[]),...(item.evidence.icpOrganizations||[])].join('、')||'未取得'}}</p><p>{{item.assetCount}} 条资产；示例 #{{item.evidence.exampleAssetIds.join('、')}}</p></details><div class="conflict-actions"><button class="button ghost compact" :disabled="busy||readOnly||statusUncertain" @click="decide(item,'accepted')">确认相关</button><button class="button ghost compact" :disabled="busy||readOnly||statusUncertain" @click="decide(item,'rejected')">排除这组线索</button></div></article>
   <article v-for="item in certificateConflicts" :key="item.kind+item.identity"><strong>证书与主机身份不一致</strong><p>{{item.hosts.join('、')}}{{item.hostCount&&item.hostCount>item.hosts.length?` 等 ${item.hostCount} 个主机`:''}}</p><p>{{item.detail}}</p><details><summary>证书标识</summary><p>{{item.identity}}</p></details><button class="button ghost compact" :disabled="busy||readOnly||statusUncertain" @click="acknowledgeCertificate(item)">我已了解，保留冲突记录</button></article>
   <p v-if="(assist?.review.run?.queueCounts?.conflict||0)>conflicts.length">更多冲突保留在完整证据记录中。</p>
  </section>
  <p v-if="notice" class="ai-notice" role="status">{{notice}}</p><p v-if="error||readError||assist?.enrichmentError" class="form-error" role="alert">{{error||readError||assist?.enrichmentError}}</p>
  <div class="discovery-secondary"><button class="text-button" :aria-expanded="showSettings" data-ai-action="options" @click="showSettings=!showSettings">{{showSettings?'收起发现设置':'发现设置'}}</button><button v-if="run||pipeline" class="text-button" :aria-expanded="showProcess" data-ai-action="process" @click="showProcess=!showProcess">{{showProcess?'收起过程与用量':'过程与用量'}}</button></div>
  <section v-if="showSettings" class="ai-details discovery-settings" aria-label="发现设置">
   <p>默认复用项目已有资产；新项目自动先采集 FOFA。只有需要更换采集条件时才修改下方选项。</p>
   <label v-if="!needsInitialCollection" class="discovery-option"><input v-model="collectFirst" type="checkbox" :disabled="!optionsEditable"/><span>重新采集 FOFA，再开始新一轮发现<small>原资产与历史保留；不勾选则沿用当前任务和证据。</small></span></label>
   <label class="field collection-targets"><span>FOFA 初始种子（可选，每行一个单位、域名、IP 或网段）</span><textarea v-model="collectionTargets" rows="3" :disabled="!optionsEditable" placeholder="例如：10086.cn&#10;留空时按目标单位搜索"/><small>{{needsInitialCollection||collectFirst?'此次采集会使用这些种子。':'仅在勾选重新采集 FOFA 后使用。'}}检索种子不自动成为探测授权。</small></label>
   <label class="discovery-option"><input v-model="includeIsolated" type="checkbox" :disabled="!optionsEditable"/><span>分析时包含隔离及回收记录<small>只读提取证据，不自动恢复资产。</small></span></label>
   <p>AI 使用当前聊天模型。Hunter 仅在明确启用后查询，可使用全部免费与权益积分；仅使用核验通过的证书指纹、域名或 CNAME，结果与扣费未知时不自动重发。</p>
  </section>
  <section v-if="showProcess&&(run||pipeline)" class="ai-details discovery-process" aria-label="发现过程与用量">
   <DiscoveryPipelineProgress v-if="pipeline" :pipeline="pipeline"/>
   <p v-if="run">当前项目 · {{run.processedAssets.toLocaleString()}} / {{run.totalAssets.toLocaleString()}} 条资产 · 已整理证据 {{run.groups.toLocaleString()}} 组</p>
   <p v-if="run">此审阅任务累计：{{run.requests}} 次请求，输入 {{run.inputTokens.toLocaleString()}} / 输出 {{run.outputTokens.toLocaleString()}} Token{{run.unknownRequests?`；${run.unknownRequests} 次结果未知，不自动重发`:''}}。{{run.model}}</p>
   <p v-if="proposalDiagnostics">{{proposalDiagnostics.networkVerified?'本次包含公开 CT 叶证书核验；证书证据不代表主机当前存活，新增探测范围仍需确认。':'本次仅核验本地证据，未联网查证企业归属。'}}</p>
   <p v-if="proposalDiagnostics?.removedNoLiveEvidence">已省略 {{proposalDiagnostics.removedNoLiveEvidence}} 项只有历史或已失效证据的范围，原记录保留。</p>
   <p v-if="heldFamilies" class="scope-held">{{heldFamilies}} 个域名家族缺少逐域绑定依据，系统自动待补证，无需逐项确认。<span v-if="proposalDiagnostics?.removedUnverified">本次拦截 {{proposalDiagnostics.removedUnverified}} 项未核验范围。</span></p>
   <details v-if="run" class="ai-details" :open="showEvidence" @toggle="showEvidence=($event.target as HTMLDetailsElement).open"><summary>完整证据记录与诊断</summary><template v-if="showEvidence"><div class="evidence-filters"><select v-model="filter" aria-label="证据记录分类"><option value="all">全部记录</option><option value="conflict">需要决定的冲突</option><option value="retain">AI 建议相关</option><option value="evidence">系统暂存的缺证线索</option><option value="exclude">建议无关</option><option value="errors">请求异常</option><option value="accepted">人工认可</option><option value="rejected">人工排除</option></select><input v-model="search" placeholder="搜索域名、证书或单位" @input="query"/></div><div class="evidence-records"><article v-for="item in data?.items" :key="item.key"><strong>{{item.kind==='certificate'?item.target:item.evidence.domains.slice(0,5).join('、')}}</strong><span>{{item.decision==='accepted'?'人工认可':item.decision==='rejected'?'人工排除':labels[item.state==='done'?item.advice:item.state]||item.state}}</span><p>{{item.queueReason||item.reason}}</p><small>{{item.assetCount}} 条资产 · 来源 {{item.evidence.sources.join(' / ')}}</small></article></div><footer><span>{{data?.total.toLocaleString()}} 组 · {{page}} / {{pages}} 页</span><button class="button ghost compact" :disabled="evidenceReading||page<=1" data-ai-action="evidence-previous" @click="loadEvidence(false,page-1)"><ChevronLeft :size="13"/>上一页</button><button class="button ghost compact" :disabled="evidenceReading||page>=pages" data-ai-action="evidence-next" @click="loadEvidence(false,page+1)">下一页<ChevronRight :size="13"/></button></footer></template></details>
  </section>
 </template>
</section>
</template>
<style scoped>
.ai-discovery{max-width:1120px;margin:0 auto;padding:24px;border:1px solid var(--line,#e2e5e9);border-radius:12px;background:var(--app-surface,#fff);font-size:13px;line-height:1.65;min-width:0;box-sizing:border-box;color:var(--text,#30343b)}.ai-discovery-heading,.heading-status,.discovery-hero,.primary-actions,.scope-heading{display:flex;justify-content:space-between;align-items:flex-start;gap:12px}.ai-discovery-heading h2{display:flex;align-items:center;gap:9px;margin:0;font-size:20px}.ai-discovery-heading p{color:var(--muted,#727984);margin:5px 0 0}.heading-status{align-items:center;flex-shrink:0}.phase{font-size:11px;border:1px solid var(--line);border-radius:5px;padding:3px 8px;color:var(--muted);white-space:nowrap}.phase.active{color:#1b6d51;background:#edf8f2;border-color:#c7e9d8}.phase.attention{color:#285d9b;background:#eff5fd;border-color:#c6d9f4}.icon-button{padding:8px!important;min-height:32px!important}.discovery-hero{margin-top:24px;align-items:flex-end}.ai-target{flex:1;min-width:0}.field{display:flex;flex-direction:column;gap:7px;min-width:0}.field>span,.saved-target>span{font-size:11px;color:var(--muted)}.field input,.collection-targets textarea,.evidence-filters input,.evidence-filters select{box-sizing:border-box;width:100%;border:1px solid var(--line);border-radius:7px;background:var(--app-surface,#fff);padding:10px 12px;color:inherit;font:inherit;min-height:40px;min-width:0}.field input:focus,.collection-targets textarea:focus,.evidence-filters input:focus{outline:2px solid color-mix(in srgb,var(--blue,#2875ed) 25%,transparent);outline-offset:1px;border-color:var(--blue,#2875ed)}.saved-target{display:flex;align-items:baseline;flex-wrap:wrap;gap:6px 12px}.saved-target>span{flex-basis:100%}.saved-target strong{font-size:18px;font-weight:600;overflow-wrap:anywhere}.text-button{display:inline-flex;align-items:center;gap:4px;border:0;padding:0;background:transparent;color:var(--blue,#2875ed);font:inherit;font-size:11px;cursor:pointer;white-space:nowrap}.text-button:disabled{color:var(--muted);cursor:default}.cancel-target{margin-top:6px}.primary-actions{align-items:center;flex-wrap:wrap;justify-content:flex-end;flex-shrink:0}.primary-actions .button{min-height:40px;white-space:nowrap}.ai-next-step{margin:12px 0 0;color:var(--muted);overflow-wrap:anywhere}.run-context{display:flex;gap:5px 14px;flex-wrap:wrap;color:var(--muted);font-size:11px;margin-top:8px;overflow-wrap:anywhere}.discovery-metrics{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));margin:20px 0 14px;border:1px solid var(--line);border-radius:8px;overflow:hidden}.discovery-metrics>div{padding:12px 16px;min-width:0}.discovery-metrics>div+div{border-left:1px solid var(--line)}.discovery-metrics dt{font-size:11px;color:var(--muted)}.discovery-metrics dd{display:flex;align-items:center;gap:12px;flex-wrap:wrap;font-size:23px;font-weight:600;line-height:1.5;font-variant-numeric:tabular-nums;margin:2px 0 0}.discovery-metrics small{font-size:10px;color:var(--muted)}.review-progress,.enrichment-summary{font-size:11px;color:var(--muted);margin:8px 0;overflow-wrap:anywhere}.review-progress progress{display:block;width:100%;height:5px;accent-color:var(--blue);margin-top:6px}.hunter-summary{display:flex;gap:7px 16px;align-items:flex-start;flex-wrap:wrap;font-size:11px;color:var(--muted);padding:12px 0;border-top:1px solid var(--line);border-bottom:1px solid var(--line)}.hunter-option,.discovery-option{display:flex;align-items:flex-start;gap:9px}.hunter-option>span{color:var(--text);font-size:12px}.hunter-option small,.discovery-option small{display:block;font-size:11px;color:var(--muted);line-height:1.6;margin-top:2px}.hunter-option input,.discovery-option input,.scope-family-choice input{width:16px;height:16px;min-height:0;margin:2px 0 0;flex:0 0 16px;accent-color:var(--blue)}.hunter-cost{margin-left:auto;font-variant-numeric:tabular-nums}.hunter-receipt,.hunter-setup{flex-basis:100%}.hunter-summary.unsettled .hunter-receipt{color:#936013}.hunter-setup .text-button{margin-left:5px}.ai-scope-confirm{padding:18px;border:1px solid #b9d1ef;border-radius:9px;background:#f9fbff;margin:20px 0}.ai-discovery h3{font-size:15px;margin:0}.scope-heading p,.scope-note,.scope-authorization{font-size:11px;color:var(--muted);margin:5px 0 10px}.scope-count{color:#37669b;font-size:11px;white-space:nowrap}.scope-preview{display:flex;gap:7px;flex-wrap:wrap;margin:12px 0}.scope-preview>span{border:1px solid #d4e0f0;background:var(--app-surface,#fff);border-radius:5px;padding:4px 8px;font-size:12px;overflow-wrap:anywhere}.scope-preview small{margin-left:6px;color:var(--muted);font-size:10px}.scope-adjust>summary,.ai-details>summary,.scope-boundary>summary,.ai-conflicts summary{cursor:pointer;color:#37669b;font-size:11px}.scope-adjust>summary{padding:5px 0}.scope-family-list{max-height:340px;overflow:auto;overscroll-behavior:contain;margin:8px 0 14px;border-top:1px solid var(--line)}.scope-family-list article{padding:12px 2px;border-bottom:1px solid var(--line)}.scope-family-choice{display:flex;align-items:flex-start;gap:9px}.scope-family-choice span{min-width:0}.scope-family-choice strong{display:block;overflow-wrap:anywhere;font-size:12px}.scope-family-choice small{display:block;color:var(--muted);font-size:11px}.scope-verification,.scope-boundary{margin:5px 0 6px 25px;font-size:11px;color:var(--muted);overflow-wrap:anywhere}.scope-verification p,.scope-boundary p{margin:3px 0}.verification-source-url{display:block;overflow-wrap:anywhere}.scope-authorization{margin-top:14px}.scope-note{margin:10px 0 0}.ai-conflicts{padding:16px;border:1px solid #ead6b7;border-radius:8px;margin:18px 0}.ai-conflicts>p,.ai-conflicts article>p{color:var(--muted);font-size:11px;margin:5px 0 10px}.ai-conflicts article{padding:12px 0;border-top:1px solid var(--line);overflow-wrap:anywhere}.ai-conflicts article strong{font-size:12px}.ai-conflicts details{font-size:11px;color:var(--muted)}.conflict-actions{display:flex;gap:8px;margin-top:9px}.ai-notice{color:#246c4e;font-size:12px}.form-error{font-size:12px;overflow-wrap:anywhere}.discovery-secondary{display:flex;gap:20px;margin-top:15px}.ai-details{border-top:1px solid var(--line);margin-top:14px;padding-top:14px;font-size:11px;color:var(--muted)}.ai-details p{margin:5px 0 12px;overflow-wrap:anywhere}.discovery-settings{display:grid;gap:12px}.collection-targets textarea{min-height:85px;resize:vertical;font-size:12px}.collection-targets small{color:var(--muted);font-size:11px}.evidence-filters{display:grid;grid-template-columns:210px minmax(0,1fr);gap:10px;margin-top:12px}.evidence-filters input,.evidence-filters select{font-size:12px;min-height:34px;padding:7px 9px}.evidence-records{max-height:380px;overflow:auto;margin:12px 0}.evidence-records article{border-top:1px solid var(--line);padding:10px 0;overflow-wrap:anywhere}.evidence-records strong{display:block;color:var(--text);font-size:12px}.evidence-records span{display:block;margin-top:4px}.evidence-records p{margin:4px 0}.ai-details footer{display:flex;align-items:center;justify-content:flex-end;gap:7px}.ai-details footer>span{margin-right:auto}.button:disabled{cursor:default}
@media(max-width:900px){.discovery-hero{align-items:flex-start;flex-direction:column}.primary-actions{justify-content:flex-start}.ai-target{width:100%}.discovery-metrics>div{padding:11px 12px}}
@media(max-width:620px){.ai-discovery{padding:16px}.ai-discovery-heading{gap:8px}.ai-discovery-heading h2{font-size:18px}.ai-discovery-heading p{font-size:11px}.heading-status{gap:6px}.saved-target strong{font-size:16px}.discovery-metrics{grid-template-columns:1fr;margin-top:16px}.discovery-metrics>div{display:grid;grid-template-columns:90px minmax(0,1fr);gap:0 10px;align-items:baseline}.discovery-metrics>div+div{border-left:0;border-top:1px solid var(--line)}.discovery-metrics dd{font-size:19px}.discovery-metrics small{grid-column:2}.scope-heading{flex-wrap:wrap}.ai-scope-confirm{padding:14px}.ai-scope-confirm>.button{width:100%;justify-content:center;white-space:normal}.primary-actions{width:100%}.primary-actions>.primary{flex:1;justify-content:center}.hunter-cost{margin-left:25px}.evidence-filters{grid-template-columns:1fr}.ai-details footer{flex-wrap:wrap}}
</style>
