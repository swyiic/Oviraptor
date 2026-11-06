<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { Search, Pause, ChevronLeft, ChevronRight } from '@lucide/vue';
import { useI18n } from '../uiPorts';
const props=defineProps<{projectId?:number}>();
const {tr}=useI18n();
type Candidate={assetId:number;host:string;link:string;isDeleted:boolean;decision:string;note:string;contentCategory:string;certificates:{fingerprint:string;anchor:string}[]};
type Batch={items:Candidate[];scanned:number;nextCursor:number;hasMore:boolean;roots:string[]};
const items=ref<Candidate[]>([]),running=ref(false),scanned=ref(0),cursor=ref(0),complete=ref(false),error=ref(''),roots=ref<string[]>([]),page=ref(1);
let generation=0,disposed=false;
const pages=computed(()=>Math.max(1,Math.ceil(items.value.length/20)));
const visible=computed(()=>items.value.slice((page.value-1)*20,page.value*20));
async function scan(){
 const projectId=props.projectId,g=generation;if(!projectId||running.value||complete.value)return complete.value;
 running.value=true;error.value='';
 try{while(running.value&&!disposed&&g===generation){
   const batch=await invoke<Batch>('review_quarantined_certificates',{projectId,cursor:cursor.value});
   if(disposed||g!==generation)return false;
   items.value.push(...batch.items);scanned.value+=batch.scanned;cursor.value=batch.nextCursor;roots.value=batch.roots;
   if(!batch.hasMore){complete.value=true;break;}
   if(items.value.length>=10000){error.value=tr('已找到至少 10,000 条复核候选，已暂停；当前列表保留，可继续。','Found at least 10,000 review candidates, so the review paused. The current list is kept; you can continue.');break;}
  }}catch(e){if(g===generation)error.value=String(e);}finally{if(g===generation)running.value=false;}
 return !disposed&&g===generation&&!error.value;
}
function reset(){generation++;running.value=false;items.value=[];scanned.value=0;cursor.value=0;complete.value=false;error.value='';roots.value=[];page.value=1;}
async function refresh(){
 // Do not turn a page refresh into the user's first full certificate review.
 // If a review is already visible, re-read it from the current local evidence.
 const hadReview=running.value||complete.value||cursor.value>0||items.value.length>0;
 reset();
 return hadReview&&props.projectId?scan():true;
}
watch(()=>props.projectId,reset);
onUnmounted(()=>{disposed=true;generation++;running.value=false;});
defineExpose({refresh,error});
</script>
<template>
 <section class="quarantine-certificate-review panel">
  <header><div><h3>{{tr('从隔离资产中查找目标证书','Find target certificates in quarantined assets')}}</h3><p>{{tr('只读取本地已保存的证书，自动分批复核。指纹对应的证书域名须全部落在项目确认范围内；不联网、不消耗积分、不修改资产或隔离规则。','Reads only locally saved certificates and reviews them in batches. Every certificate domain behind a fingerprint must fall within the project’s confirmed scope. No network access, no points spent, and no changes to assets or quarantine rules.')}}</p></div><button v-if="!running" class="button secondary compact" :disabled="!projectId||complete" @click="scan"><Search :size="15"/>{{complete?tr('本轮复核完成','Review complete'):cursor?tr('继续本地复核','Continue local review'):tr('开始本地证书复核','Start local certificate review')}}</button><button v-else class="button secondary compact" @click="running=false"><Pause :size="15"/>{{tr('暂停复核','Pause review')}}</button></header>
  <p v-if="roots.length" class="review-scope">{{tr(`匹配范围：${roots.join('、')}`,`Matching scope: ${roots.join(', ')}`)}}</p>
  <p v-if="scanned||running" role="status">{{running?tr('正在复核','Reviewing'):complete?tr('复核完成','Review complete'):tr('已暂停','Paused')}} · {{tr(`已检查 ${scanned.toLocaleString()} 条隔离 / 回收记录 · 找到 ${items.length.toLocaleString()} 条证书匹配候选`,`Checked ${scanned.toLocaleString()} quarantined / trashed records · Found ${items.length.toLocaleString()} certificate match candidates`)}}</p>
  <p class="review-note">{{tr('证书匹配只提示值得复核，不证明页面分类错误。请在下方资产表按入口定位，检查命中规则与内容后再恢复指定资产；人工排除和回收记录也只展示，不自动恢复。','A certificate match only flags an asset worth reviewing; it doesn’t prove the page was misclassified. Find the entry in the asset table below and check the matched rule and content before restoring a specific asset. Manual exclusions and trashed records are only shown, never restored automatically.')}}</p>
  <div v-if="items.length" class="review-table"><table><thead><tr><th>{{tr('资产入口','Asset entry')}}</th><th>{{tr('证书域名与指纹','Certificate domains and fingerprints')}}</th><th>{{tr('当前隔离依据','Current quarantine basis')}}</th></tr></thead><tbody><tr v-for="item in visible" :key="item.assetId"><td><strong>#{{item.assetId}}</strong><code>{{item.link||item.host}}</code></td><td><div v-for="cert in item.certificates" :key="cert.fingerprint"><strong>{{cert.anchor}}</strong><code>{{cert.fingerprint}}</code></div></td><td>{{item.contentCategory||item.decision}}<small>{{item.isDeleted?tr('已在回收站 · ','In trash · '):''}}{{item.note||tr('检查原始内容与分类证据','Check the original content and classification evidence')}}</small></td></tr></tbody></table></div>
  <footer v-if="items.length"><span>{{tr(`第 ${page} / ${pages} 页`,`Page ${page} / ${pages}`)}}</span><button class="button ghost compact" :disabled="page<=1" @click="page--" :aria-label="tr('上一页复核候选','Previous page of review candidates')"><ChevronLeft :size="15"/></button><button class="button ghost compact" :disabled="page>=pages" @click="page++" :aria-label="tr('下一页复核候选','Next page of review candidates')"><ChevronRight :size="15"/></button></footer>
  <p v-if="complete&&!items.length">{{tr('本地尚无符合条件的证书。缺少证书不代表资产无关；可在任务中心对确认域名范围内资产补采，再复核。','No matching certificates are stored locally. A missing certificate doesn’t mean an asset is unrelated; re-collect assets within the confirmed domain scope from the task center, then review again.')}}</p><button v-if="complete" class="button ghost compact" @click="reset">{{tr('重新检查当前数据','Re-check current data')}}</button>
  <p v-if="error" role="alert" class="review-error">{{error}}</p>
 </section>
</template>
<style scoped>
.quarantine-certificate-review{padding:20px;margin-bottom:20px;min-width:0;font-size:var(--text-body);line-height:1.65}.quarantine-certificate-review header{display:flex;justify-content:space-between;gap:20px;align-items:flex-start}h3{font-size:var(--text-heading);margin:0 0 6px}p{color:var(--muted);margin:8px 0}.button{display:inline-flex;align-items:center;justify-content:center;gap:8px;white-space:nowrap;font:inherit}.review-note{padding:10px 12px;background:var(--app-bg,var(--ui-surface-raised));border-radius:var(--radius-md)}.review-table{overflow:auto}table{width:100%;border-collapse:collapse;table-layout:fixed}th,td{padding:10px;text-align:left;border-bottom:1px solid var(--line);vertical-align:top;overflow-wrap:anywhere}th{color:var(--muted)}code,small{display:block;font-size:var(--text-meta);overflow-wrap:anywhere}small{color:var(--muted)}footer{display:flex;justify-content:flex-end;gap:8px;align-items:center;margin-top:12px}.review-error{color:var(--danger,color-mix(in srgb, var(--red) 82%, var(--ui-ink)))}@media(max-width:700px){header{flex-direction:column}table{min-width:560px}}
</style>
