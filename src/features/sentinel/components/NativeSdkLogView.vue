<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import {computed} from "vue";
import type {SdkLogRow} from "../execution/nativeSdkLogContract";
import {useNativeSdkLog} from "../execution/useNativeSdkLog";
import {useI18n} from "../../../i18n";
const props=defineProps<{scanId:string;attempt?:number;active?:boolean}>();
const {tr}=useI18n();
const {replay,loading,readFailed,connectionUnavailable,refresh}=useNativeSdkLog(
 ()=>props.active!==false&&props.scanId?{scanId:props.scanId,attempt:props.attempt??0}:undefined);
const stageLabel=(value?:string)=>( ({prepared:tr("原调用已准备","Original call prepared"),sent:tr("已发起请求","Request sent"),response_received:tr("HTTP 响应已读完","HTTP response fully read"),cost_saved:tr("原费用记录已保存","Original cost record saved"),validated:tr("SDK 检查已通过","SDK checks passed"),terminal:tr("SDK 入口已终结","SDK entry finalized")} as Record<string,string>)[value??""]??tr("阶段需核对","Stage needs checking"));
const domainLabel=(value?:string)=>( ({root:tr("Root 模型调用","Root model call"),specialist:tr("独立子任务调用","Independent subtask call"),source_round:tr("源码工具轮 SDK 调用","Source tool round SDK call"),mapper_loop:tr("Mapper 受控工具轮 SDK 调用","Mapper controlled tool round SDK call")} as Record<string,string>)[value??""]??tr("调用归属需核对","Call owner needs checking"));
const terminalLabel=(value?:string)=>( ({returned:tr("原结果已返回","Original result returned"),withheld:tr("结果未交付后续工作","Result withheld from follow-up work"),uncertain:tr("结果未知 · 原占用保留","Result unknown · original reservation kept"),unsent:tr("尚未发出","Not sent yet"),failed:tr("入口状态需核对","Entry state needs checking")} as Record<string,string>)[value??""]??tr("尚无终结记录","No final record yet"));
const costLabel=(value?:string)=>( ({received:tr("已保存收到响应的原费用事实","Original cost facts saved for a received response"),uncertain:tr("已保存未知结果的原费用事实","Original cost facts saved for an unknown result"),unsent:tr("已保存未发出的原费用事实","Original cost facts saved for an unsent request")} as Record<string,string>)[value??""]??"");
const calls=computed(()=>{
 const grouped=new Map<string,{ownerId:string;rows:SdkLogRow[];last:SdkLogRow}>();
 for(const row of replay.value.rows){
  const call=grouped.get(row.ownerId);
  if(call){call.rows.push(row);call.last=row;}
  else grouped.set(row.ownerId,{ownerId:row.ownerId,rows:[row],last:row});
 }
 return [...grouped.values()].reverse();
});
</script>
<template>
 <section class="native-sdk-log" :aria-label="tr('原生 SDK 调用阶段','Native SDK call stages')">
  <header><div><strong>{{tr(`SDK 调用诊断 · #${replay.attempt??props.attempt??0}`,`SDK call diagnostics · #${replay.attempt??props.attempt??0}`)}}</strong></div><button v-refresh-feedback="{scope:scanId+':'+attempt,busy:loading,error:readFailed}" class="button ghost compact" type="button" :disabled="loading" @click="refresh">{{tr('刷新诊断','Refresh diagnostics')}}</button></header>
  <p class="sdk-boundary">{{tr('这里只显示已提交的安全阶段，不保存模型内容。请求已发起不等于服务端已接受；SDK 终结不代表智能体、工具、证据审核或任务已完成。',"Only committed, safe stages are shown here; model content is not stored. A sent request doesn't mean the server accepted it, and a finalized SDK entry doesn't mean the agent, tools, evidence review, or task is complete.")}}</p>
  <p v-if="connectionUnavailable" role="status">{{tr('实时订阅暂不可用，正在补读已提交记录并重连。','Live subscription unavailable; catching up on committed records and reconnecting.')}}</p>
  <p v-if="readFailed" role="alert">{{tr('此轮 SDK 记录无法核验，保留最后一次提交内容；不能据此判断调用成功。',"This attempt's SDK records can't be verified; the last committed content is kept. Don't take it as proof that calls succeeded.")}}</p>
  <p v-if="replay.hasGap" role="status">{{tr('SDK 阶段采集存在缺口；原调用结果和费用以独立业务回执为准。','SDK stage capture has gaps; the separate business receipts are authoritative for original call results and costs.')}}</p>
  <p v-if="replay.incompleteOwners.length" role="status">{{tr(`${replay.incompleteOwners.length} 个原调用尚无完整诊断终结记录；可能仍在运行或采集已中断，不能视为已完成。`,`${replay.incompleteOwners.length} original calls have no complete final diagnostic record; they may still be running or capture may have stopped, so they can't be treated as complete.`)}}</p>
  <p v-if="replay.metadataTruncated" role="status">{{tr('诊断归属或缺口列表已达到显示上限，未展示项目不能视为已完成。',"The diagnostic owner or gap list reached its display limit; items not shown can't be treated as complete.")}}</p>
  <p v-if="replay.evicted" role="status">{{tr(`当前显示最近 300 条阶段，${replay.evicted} 条已移出视图，完整记录仍可按游标读取。`,`Showing the latest 300 stages; ${replay.evicted} have left the view. The full record can still be read by cursor.`)}}</p>
  <p v-if="replay.more||loading" role="status">{{tr('正在恢复已提交的 SDK 阶段…','Restoring committed SDK stages…')}}</p>
  <div v-if="replay.rows.length" class="sdk-rows" role="region" :aria-label="tr('已提交 SDK 阶段','Committed SDK stages')" tabindex="0">
   <article v-for="call in calls" :key="call.ownerId" :data-state="call.last.terminalState||'open'">
    <header class="sdk-call-heading">
     <div><b>{{domainLabel(call.last.domain)}}</b><span>{{tr(`模型轮次 ${call.last.round}`,`Model round ${call.last.round}`)}}</span></div>
     <span class="sdk-call-state">{{call.last.stage==='terminal'?terminalLabel(call.last.terminalState):stageLabel(call.last.stage)}}</span>
    </header>
    <ol class="sdk-stage-trail" :aria-label="tr('此调用已提交的阶段','Committed stages for this call')">
     <li v-for="row in call.rows" :key="row.sequence"><span class="sdk-stage-dot" aria-hidden="true"></span><div><b>{{stageLabel(row.stage)}}</b><time>{{row.time}}</time></div></li>
    </ol>
    <small v-if="call.last.costPhase" class="sdk-cost">{{costLabel(call.last.costPhase)}} · {{tr('不等于最终账单金额','Not the final billed amount')}}</small>
    <details><summary>{{tr('原归属详情','Original owner details')}}</summary><code>{{call.last.domain}} · {{call.last.stage}} · {{call.last.sequence}} / {{call.last.ordinal}}<br>{{call.ownerId}}<br>{{call.last.dispatchKey}}<br>run {{call.last.runId}}<br>worker {{call.last.workerId??tr('无子工作者','No child worker')}}<br>{{call.last.costPhase}} · {{call.last.terminalState}}</code></details>
   </article>
  </div>
  <details v-if="replay.gaps.length"><summary>{{tr('查看已保存的采集缺口','View saved capture gaps')}}</summary><p v-for="gap in replay.gaps" :key="gap.ownerId">{{stageLabel(gap.failedStage)}} · {{tr(`采集写入失败（${gap.code}）`,`Capture write failed (${gap.code})`)}}<br><code>{{gap.ownerId}} · {{tr(`已保存 ${gap.afterOrdinal} 个阶段`,`${gap.afterOrdinal} stages saved`)}} · {{gap.time}}</code></p></details>
  <p v-if="!loading&&!readFailed&&!replay.available">{{tr('此轮尚无可核验的 SDK 诊断归属，不能推断是否已经调用模型。',"No verifiable SDK diagnostic owner for this attempt yet; it can't be inferred whether the model was called.")}}</p>
 </section>
</template>
<style scoped>
.native-sdk-log{display:grid;gap:12px;min-width:0;color:var(--app-ink,var(--ui-ink))}
.native-sdk-log header{display:flex;justify-content:space-between;align-items:center;gap:10px;flex-wrap:wrap}
.native-sdk-log header>div{display:grid;gap:6px}.native-sdk-log strong{font-size:var(--text-heading)}
.native-sdk-log p{margin:0;font-size:var(--text-body);line-height:1.65;color:var(--app-muted,var(--ui-muted));overflow-wrap:anywhere}
.native-sdk-log [role="alert"]{color:#d16666}.sdk-boundary{padding:12px 14px;border:1px solid var(--line,var(--ui-line));border-radius:var(--radius-md);background:var(--app-surface,var(--ui-surface))}
.sdk-rows{display:grid;gap:12px;max-height:440px;overflow:auto;scrollbar-width:thin;padding:2px}
.sdk-rows article{min-width:0;padding:16px;border:1px solid var(--line,var(--ui-line));border-radius:var(--radius-lg);background:var(--app-surface,var(--ui-surface));font-size:var(--text-body)}
.sdk-call-heading>div{display:flex!important;align-items:baseline;gap:10px!important}.sdk-call-heading b{font-weight:650}
.sdk-call-heading span,.sdk-cost{color:var(--app-muted,var(--ui-muted));font-size:var(--text-meta)}
.sdk-call-state{padding:4px 10px;border-radius:var(--radius-sm);background:color-mix(in srgb,var(--app-muted,#717780) 10%,transparent)}
[data-state="returned"] .sdk-call-state{color:var(--app-ink,var(--ui-ink))}
[data-state="uncertain"] .sdk-call-state,[data-state="withheld"] .sdk-call-state{color:#b38140;background:color-mix(in srgb,#b38140 12%,transparent)}
.sdk-stage-trail{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px 20px;margin:16px 0;padding:0;list-style:none}
.sdk-stage-trail li{display:flex;gap:10px;align-items:flex-start;min-width:0}.sdk-stage-trail li>div{display:grid;gap:3px;min-width:0}
.sdk-stage-trail b{font-size:var(--text-meta);font-weight:550}.sdk-stage-dot{width:6px;height:6px;flex:0 0 6px;margin-top:6px;border-radius:50%;background:var(--app-muted,#717780)}
.sdk-stage-trail time{color:var(--app-muted,var(--ui-muted));font-size:var(--text-micro);overflow-wrap:anywhere}.sdk-cost{display:block;margin:0 0 10px;line-height:1.6}
.native-sdk-log details{color:var(--app-muted,var(--ui-muted));font-size:var(--text-meta);line-height:1.6}.native-sdk-log summary{cursor:pointer}
.native-sdk-log code{display:block;overflow-wrap:anywhere;white-space:pre-wrap;font-size:var(--text-micro);margin-top:8px}
.sdk-rows:focus-visible,.native-sdk-log summary:focus-visible{outline:2px solid var(--blue,#2878ff);outline-offset:3px}
@media(max-width:600px){.sdk-stage-trail{grid-template-columns:1fr}.sdk-rows article{padding:14px}.sdk-call-heading{align-items:flex-start!important}}
</style>
