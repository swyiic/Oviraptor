<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from '../uiPorts';
const props=defineProps<{projectId?:number;suggestedDomain?:string}>();
const emit=defineEmits<{changed:[]}>();
const {tr}=useI18n();
type Preview={domain:string;domainLeads:number;relatedFingerprints:number;protectedSharedFingerprints:number;total:number;scopeConflict:boolean;digest:string;samples:{host:string;source:string;reference:string}[]};
const domain=ref(''),domains=ref<string[]>([]),preview=ref<Preview>(),busy=ref(false),opened=ref(false),error=ref(''),notice=ref(''),mode=ref<'apply'|'restore'>('apply');
let generation=0,disposed=false;
async function load(g=generation){if(!props.projectId)return;try{const value=await invoke<{domains:string[]}>('get_discovery_domain_exclusions',{projectId:props.projectId});if(!disposed&&g===generation)domains.value=value.domains;}catch(e){if(g===generation)error.value=String(e);}}
async function open(){if(!opened.value){opened.value=true;domain.value=props.suggestedDomain?.trim()||'';await load();}else opened.value=false;}
async function inspect(action:'apply'|'restore'='apply',value=domain.value){
 const projectId=props.projectId,g=generation;if(!projectId||busy.value||!value.trim())return;
 busy.value=true;error.value='';notice.value='';preview.value=undefined;mode.value=action;domain.value=value.trim();
 try{const result=await invoke<Preview>('manage_discovery_domain_exclusion',{input:{projectId,domain:domain.value,action:'preview',expectedDigest:null}});if(!disposed&&g===generation)preview.value=result;}
 catch(e){if(g===generation)error.value=String(e);}finally{if(g===generation)busy.value=false;}
}
async function apply(){
 const projectId=props.projectId,g=generation,p=preview.value;if(!projectId||busy.value||!p)return;
 busy.value=true;error.value='';
 try{await invoke('manage_discovery_domain_exclusion',{input:{projectId,domain:p.domain,action:mode.value,expectedDigest:p.digest}});if(disposed||g!==generation)return;notice.value=mode.value==='apply'?tr(`已排除 ${p.domain} 及其子域，默认线索列表已隐藏，后续补查会重新检查排除规则。原始资产与历史保留。`,`Excluded ${p.domain} and its subdomains. They are hidden from the default lead list, and later lookups re-check exclusion rules. Original assets and history are kept.`):tr(`已恢复 ${p.domain} 的排除规则，线索仍需通过付费准入检查。`,`Removed the exclusion for ${p.domain}. Its leads still have to pass the paid-query eligibility check.`);preview.value=undefined;await load(g);if(g===generation)emit('changed');}
 catch(e){if(g===generation){error.value=String(e);preview.value=undefined;}}finally{if(g===generation)busy.value=false;}
}
watch(domain,()=>{preview.value=undefined;},{flush:'sync'});
watch(()=>props.projectId,()=>{generation++;opened.value=false;busy.value=false;preview.value=undefined;domain.value='';domains.value=[];error.value='';notice.value='';});
onUnmounted(()=>{disposed=true;generation++;});
</script>
<template>
 <section class="discovery-exclusions">
  <button type="button" class="button secondary compact" :aria-expanded="opened" :disabled="!projectId" @click="open">{{tr('按域名批量排除无关线索','Bulk-exclude unrelated leads by domain')}}</button>
  <div v-if="opened" class="exclusion-body">
   <p>{{tr('确认某个域名与目标无关后，可将该域名及其子域、仅由这些来源产生的证书指纹移出默认线索列表。其他独立来源使用的共享指纹会保留；资产、证书原文及历史记录保留，可恢复。','Once a domain is confirmed unrelated to the target, you can remove it, its subdomains, and certificate fingerprints that come only from them from the default lead list. Shared fingerprints used by other independent sources stay. Assets, raw certificates and history are kept and can be restored.')}}</p>
   <div class="exclusion-input"><label class="field"><span>{{tr('已确认无关的域名','Domain confirmed as unrelated')}}</span><input v-model="domain" :disabled="busy" placeholder="unrelated.example.com" autocomplete="off" /></label><button class="button secondary compact" :disabled="busy||!domain.trim()" @click="inspect()">{{tr('预览排除范围','Preview exclusion scope')}}</button></div>
   <div v-if="preview" class="exclusion-preview" role="status">
    <strong>{{mode==='restore'?tr(`恢复排除规则：${preview.domain}`,`Remove exclusion: ${preview.domain}`):tr(`排除域名及子域：${preview.domain}`,`Exclude domain and subdomains: ${preview.domain}`)}}</strong>
    <p>{{tr(`域名类线索 ${preview.domainLeads.toLocaleString()} 条 · 关联指纹 ${preview.relatedFingerprints.toLocaleString()} 条 · 有其他来源的共享指纹保留 ${preview.protectedSharedFingerprints.toLocaleString()} 条`,`${preview.domainLeads.toLocaleString()} domain leads · ${preview.relatedFingerprints.toLocaleString()} related fingerprints · ${preview.protectedSharedFingerprints.toLocaleString()} shared fingerprints kept for other sources`)}}</p>
    <p v-if="preview.scopeConflict" class="exclusion-warning">{{tr('这个域名与当前确认范围重叠。排除优先于确认范围，排除后相关补查也会停止。','This domain overlaps the current confirmed scope. Exclusion takes precedence over confirmed scope, so related follow-up lookups stop too.')}}</p>
    <p>{{tr('仅作用于当前项目。已发出的 Hunter 请求无法撤销；后续分页会检查排除状态。','Applies to this project only. Hunter requests already sent can’t be undone; later pages check the exclusion status.')}}</p>
    <ul v-if="preview.samples.length"><li v-for="(item,index) in preview.samples" :key="index"><code>{{item.host}}</code><small>{{item.source}} · {{item.reference}}</small></li></ul>
    <button class="button secondary compact" :disabled="busy" @click="apply">{{mode==='restore'?tr('确认恢复这条排除规则','Confirm removing this exclusion'):tr('确认排除预览中的线索','Confirm excluding the previewed leads')}}</button>
   </div>
   <p v-if="busy" role="status">{{tr('正在处理本地线索，请勿重复点击…','Processing local leads. Please don’t click again…')}}</p>
   <div v-if="domains.length" class="exclusion-rules"><strong>{{tr('当前项目已排除','Excluded in this project')}}</strong><div v-for="item in domains" :key="item"><code>{{item}}</code><button class="button ghost compact" :disabled="busy" @click="inspect('restore',item)">{{tr('预览恢复','Preview removal')}}</button></div></div>
   <p v-if="notice" role="status">{{notice}}</p><p v-if="error" role="alert" class="exclusion-error">{{error}}</p>
  </div>
 </section>
</template>
<style scoped>
.discovery-exclusions{margin:12px 0}.exclusion-body{margin-top:12px;padding:14px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--app-bg,var(--ui-surface-raised));font-size:var(--text-body);line-height:1.7;overflow-wrap:anywhere}.exclusion-body p{margin:6px 0;color:var(--muted)}.exclusion-input{display:flex;align-items:flex-end;gap:10px;margin:12px 0}.exclusion-input .field{flex:1;margin:0;min-width:0}.button{display:inline-flex;gap:8px;align-items:center;justify-content:center;white-space:nowrap;font:inherit}.exclusion-preview{padding:12px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--app-surface,var(--ui-surface))}.exclusion-preview .exclusion-warning{color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink))}.exclusion-preview ul{max-height:150px;overflow:auto;padding-left:16px}.exclusion-preview li{padding:3px 0}.exclusion-preview small{display:block;color:var(--muted)}code{overflow-wrap:anywhere;font-size:var(--text-meta)}.exclusion-rules{margin-top:14px}.exclusion-rules>div{display:flex;align-items:center;justify-content:space-between;gap:10px;border-bottom:1px solid var(--line);padding:6px 0}.exclusion-body .exclusion-error{color:var(--danger,color-mix(in srgb, var(--red) 82%, var(--ui-ink)))}@media(max-width:650px){.exclusion-input{align-items:stretch;flex-direction:column}}
</style>
