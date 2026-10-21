<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue';
import { invoke } from '../uiPorts';
export type HunterExpansion = { id:string;kind:string;target:string;query:string;status:string;pageSize:number;nextPage:number;total:number|null;received:number;uniqueRows:number;rejected:number;spentPoints:number;costUnknown:boolean;reason:string;account?:{rest_free_point?:number;rest_equity_point?:number};startDate?:string;endDate?:string;pages?:{page:number;state:string;points:number|null;consumeQuota:string;restQuota:string;returned:number;accepted:number;rejected:number}[] };
const props=defineProps<{projectId?:number;items?:HunterExpansion[]}>();const emit=defineEmits<{changed:[]}>();
const busy=ref(''),error=ref('');let generation=0,disposed=false;
watch(()=>props.projectId,()=>{generation++;busy.value='';error.value='';});onUnmounted(()=>{disposed=true;generation++;});
const labels:Record<string,string>={running:'正在自动翻页',stopping:'正在暂停',paused:'已暂停，可继续',completed:'已遍历报告的全部分页',unknown:'远端结果 / 扣费待核对',partial:'有未完成或未验证部分',quota:'积分或额度不足',provider_limit:'平台限制，未完成',interrupted:'已中断，保留游标'};
async function act(action:'stop'|'resume',id:string){if(!props.projectId||busy.value)return;const p=props.projectId,g=generation;busy.value=id;error.value='';try{await invoke(`${action}_hunter_expansion`,{projectId:p,queryId:id});if(!disposed&&g===generation)emit('changed');}catch(e){if(!disposed&&g===generation)error.value=String(e);}finally{if(!disposed&&g===generation)busy.value='';}}
</script>
<template>
<section v-if="items?.length" class="hunter-progress" data-testid="hunter-expansion-progress">
<h4>Hunter 精确关联 · 分页与积分</h4>
<article v-for="item in items" :key="item.id" :data-query-id="item.id">
<header><strong>{{item.target}}</strong><span :data-hunter-status="item.status">{{labels[item.status]||item.status}}</span></header>
<p><code>{{item.query}}</code></p><p v-if="item.startDate">时间范围：{{item.startDate}} 至 {{item.endDate}}</p>
<p>累计返回 {{item.received}} 条（含重复） · 本查询去重后 {{item.uniqueRows}} 条 · 平台报告 {{item.total??'未知'}} 条 · 下一页 {{item.nextPage}}<span v-if="item.pageSize">（每页 {{item.pageSize}} 条）</span></p>
<p><strong>已核对消耗 {{item.spentPoints}} 积分</strong><span v-if="item.costUnknown" class="form-error"> · 另有消耗未确认，不能当作 0</span> · 免费余额 {{item.account?.rest_free_point??'未报告'}} · 权益余额 {{item.account?.rest_equity_point??'未报告'}}</p>
<p role="status">{{item.reason}}</p><p v-if="item.rejected" class="form-error">{{item.rejected}} 条记录未通过精确关联 / 身份核对，原始页已保留。</p>
<div class="hunter-actions"><button v-if="['running','stopping'].includes(item.status)" class="button danger compact" :disabled="!!busy||item.status==='stopping'" @click="act('stop',item.id)">暂停自动翻页</button><button v-else-if="['paused','quota','partial','interrupted'].includes(item.status)" class="button secondary compact" :disabled="!!busy" @click="act('resume',item.id)">从保存的游标继续</button></div>
<details v-if="item.pages?.length"><summary>最近 20 页回执（所有已收页均保留）</summary><div class="hunter-pages"><table><thead><tr><th>页码</th><th>状态</th><th>返回 / 新增</th><th>积分消耗</th><th>平台余额回执</th></tr></thead><tbody><tr v-for="page in item.pages" :key="page.page"><td>{{page.page}}</td><td>{{page.state}}</td><td>{{page.state==='sending'?'未确认':`${page.returned} / ${page.accepted}`}}</td><td>{{page.consumeQuota||(page.points==null?'未确认':page.points)}}</td><td>{{page.restQuota||'未报告'}}</td></tr></tbody></table></div></details>
</article><p v-if="error" class="form-error" role="alert">{{error}}</p>
</section>
</template>
<style scoped>.hunter-progress{margin-top:16px}.hunter-progress article{padding:14px;border:1px solid var(--line,#ddd);border-radius:9px;margin:10px 0}.hunter-progress header{display:flex;gap:12px;justify-content:space-between;overflow-wrap:anywhere}.hunter-progress p{font-size:12px;margin:8px 0;overflow-wrap:anywhere}.hunter-progress header span{color:var(--muted);font-size:11px;flex-shrink:0}.hunter-progress header [data-hunter-status=completed]{color:#18734d}.hunter-progress header [data-hunter-status=unknown],.hunter-progress header [data-hunter-status=partial]{color:#9b5c08}.hunter-actions{display:flex;gap:8px;margin:8px 0}.hunter-pages{overflow:auto}.hunter-pages table{width:100%;font-size:11px;border-collapse:collapse}.hunter-pages td,.hunter-pages th{padding:7px;border-bottom:1px solid var(--line,#ddd);text-align:left}</style>
