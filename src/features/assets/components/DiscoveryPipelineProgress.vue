<script setup lang="ts">
import {computed} from 'vue';
import type {DiscoveryPipeline} from '../../agentHost/discoveryTypes';
const props=defineProps<{pipeline:DiscoveryPipeline;compact?:boolean}>();
const steps=[{key:'collecting',label:'FOFA 首采'},{key:'local_analysis',label:'AI 核验'},{key:'free_discovery',label:'免费发现'},{key:'hunter',label:'Hunter 精确补查'},{key:'steady',label:'去重与回流'}];
const phaseIndex=computed(()=>({collecting:0,local_analysis:1,passive_evidence:1,awaiting_scope:1,free_discovery:2,hunter:3,steady:4,waiting_storage:-1} as Record<string,number>)[props.pipeline.phase]??-1);
const stateLabel=computed(()=>({collecting:'正在采集初始资产',local_analysis:'整理并核验归属证据',passive_evidence:'补充被动证据',awaiting_scope:'等待集中确认新增范围',free_discovery:'已确认范围内自动发现',hunter:'精确关联并自动翻页',steady:'本轮回流已保存',blocked:'已保留进度，需要处理',paused:'已暂停，可从断点继续',waiting_storage:'等待本地存储，自动重试'} as Record<string,string>)[props.pipeline.phase]||props.pipeline.phase);
const metrics=computed(()=>Object.entries({collectedAssets:'项目累计记录',uniqueAssets:'累计唯一资产记录',verifiedGroups:'核验通过',pendingEvidenceGroups:'自动待补证',conflicts:'证据冲突',freeCompleted:'免费补证',hunterAdded:'Hunter 入库记录',roundAdded:'本流程新增'}).filter(([key])=>Number.isFinite(props.pipeline.counts?.[key])).map(([key,label])=>({key,label,value:props.pipeline.counts[key]})));
const knownCost=computed(()=>typeof props.pipeline.hunter?.spentPoints==='number'&&Number.isFinite(props.pipeline.hunter.spentPoints));
const organizationEvidence=computed(()=>props.pipeline.organizationEvidence);
function checkedTime(value:number){return Number.isFinite(value)&&value>0?new Date(value*1000).toLocaleString('zh-CN',{hour12:false}):'';}
</script>
<template>
 <section class="discovery-pipeline" :class="{compact}" aria-label="资产发现完整流程" data-testid="discovery-pipeline">
  <ol class="pipeline-steps"><li v-for="(step,index) in steps" :key="step.key" :class="{current:phaseIndex===index,passed:phaseIndex>index}" :aria-current="phaseIndex===index?'step':undefined"><span>{{index+1}}</span>{{step.label}}</li></ol>
  <div class="pipeline-status" role="status"><strong>第 {{pipeline.round||1}} 轮 · {{stateLabel}}</strong><span v-if="pipeline.collectionRunId">FOFA 任务 #{{pipeline.collectionRunId}}</span></div>
  <p v-if="pipeline.reason" class="pipeline-reason">{{pipeline.reason}}</p>
  <dl v-if="metrics.length" class="pipeline-metrics"><div v-for="metric in metrics" :key="metric.key"><dt>{{metric.label}}</dt><dd>{{metric.value.toLocaleString()}}</dd></div></dl>
  <div class="pipeline-cost" :class="{unknown:pipeline.hunter?.costUnknown}" role="status">
   <template v-if="pipeline.allowHunter"><strong>Hunter</strong><span>{{pipeline.hunter?.queries||0}} 个精确查询 · {{pipeline.hunter?.pages||0}} 页</span><span v-if="knownCost">已知消耗 {{pipeline.hunter.spentPoints!.toLocaleString()}} 积分</span><span v-if="pipeline.hunter?.costUnknown">部分扣费或请求结果未知，已暂停；不会自动重发。</span><span v-else-if="!knownCost">积分回执尚未返回</span></template>
   <template v-else>Hunter 未启用 · 不消耗测绘积分</template>
  </div>
  <details v-if="organizationEvidence?.supported" class="organization-evidence"><summary>目标单位关系来源 · {{organizationEvidence.networkVerified?'已核验公开正文':organizationEvidence.names?.length?'已有缓存证据':'等待核实'}}<span v-if="organizationEvidence.checkedAt"> · {{checkedTime(organizationEvidence.checkedAt)}}</span></summary><p v-if="organizationEvidence.names?.length">已核对法定名称：{{organizationEvidence.names.join('、')}}</p><p v-if="organizationEvidence.sourceUrl" class="organization-source">{{organizationEvidence.sourceUrl}}</p><p v-if="organizationEvidence.error" class="organization-error">{{organizationEvidence.error}}</p><p>单位关系仅作为归属核验依据，不会因此新增域名范围或发起目标探测。</p></details>
 </section>
</template>
<style scoped>
.discovery-pipeline{margin:16px 0;border:1px solid var(--line,#e2e5e9);border-radius:8px;padding:14px;background:var(--app-bg,#fafbfc);min-width:0;overflow-wrap:anywhere}.pipeline-steps{display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:8px;list-style:none;padding:0;margin:0 0 14px}.pipeline-steps li{display:flex;align-items:center;gap:6px;font-size:11px;color:var(--muted,#727984);min-width:0;line-height:1.5}.pipeline-steps li>span{display:grid;place-items:center;flex:0 0 19px;width:19px;height:19px;border:1px solid var(--line,#e2e5e9);border-radius:50%;font-size:10px;background:var(--app-surface,#fff)}.pipeline-steps li.current{color:var(--blue,#2875ed);font-weight:600}.pipeline-steps li.current>span{border-color:currentColor}.pipeline-steps li.passed{color:#287358}.pipeline-status{display:flex;gap:8px 16px;align-items:center;flex-wrap:wrap;font-size:12px}.pipeline-status>span{font-size:11px;color:var(--muted)}.pipeline-reason{margin:6px 0 0;font-size:11px;color:var(--muted)}.pipeline-metrics{display:flex;flex-wrap:wrap;gap:8px 20px;margin:12px 0 0}.pipeline-metrics>div{display:flex;gap:7px;font-size:11px;align-items:baseline}.pipeline-metrics dt{color:var(--muted)}.pipeline-metrics dd{margin:0;font-weight:600;font-size:13px}.pipeline-cost{display:flex;gap:5px 12px;flex-wrap:wrap;border-top:1px solid var(--line);padding-top:9px;margin-top:11px;color:var(--muted);font-size:11px}.pipeline-cost.unknown{color:#936013}.compact{padding:11px}.compact .pipeline-steps{gap:6px}@media(max-width:760px){.pipeline-steps{grid-template-columns:repeat(3,minmax(0,1fr));gap:9px}.pipeline-metrics{gap:7px 14px}}@media(max-width:440px){.pipeline-steps{grid-template-columns:repeat(2,minmax(0,1fr))}.discovery-pipeline{padding:11px}}
.organization-evidence{margin-top:10px;font-size:11px;color:var(--muted);border-top:1px solid var(--line);padding-top:9px}.organization-evidence summary{cursor:pointer}.organization-evidence p{margin:5px 0}.organization-source{overflow-wrap:anywhere}.organization-evidence .organization-error{color:#936013}
</style>
