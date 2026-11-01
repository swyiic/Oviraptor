<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { onBeforeUnmount, ref, watch } from 'vue';
import { ShieldCheck } from '@lucide/vue';
import { useI18n } from '../../../i18n';
import type { AgentHostChatClient } from '../chatClient';
import type { AgentAccessMode, AgentAccessPolicy } from '../types';
const props=defineProps<{client:AgentHostChatClient;projectId?:number;runtimeIdentity?:string;disabled?:boolean;compact?:boolean}>();
const emit=defineEmits<{changed:[policy:AgentAccessPolicy]}>();
const {tr}=useI18n();
const policy=ref<AgentAccessPolicy>(),loading=ref(false),saving=ref(false),error=ref('');
let epoch=0,disposed=false;
async function load(){
 const projectId=props.projectId,seq=++epoch;policy.value=undefined;error.value='';loading.value=!!projectId;
 if(!projectId)return;
 try{const value=await props.client.accessPolicy(projectId);if(disposed||seq!==epoch||projectId!==props.projectId)return;policy.value=value;emit('changed',value);}
 catch{if(seq===epoch)error.value=tr('权限模式读取失败，请重试','Could not load permission mode. Retry.');}
 finally{if(seq===epoch)loading.value=false;}
}
async function change(event:Event){
 const select=event.target as HTMLSelectElement,mode=select.value as AgentAccessMode,current=policy.value,projectId=props.projectId;
 select.value=current?.mode||'ask';
 if(!current||!projectId||saving.value||props.disabled||mode===current.mode)return;
 const seq=epoch;saving.value=true;error.value='';
 try{const value=await props.client.setAccessPolicy({projectId,mode,expectedRevision:current.revision,expectedRuntimeIdentity:current.runtimeIdentity,operatorConfirmed:mode==='full'});
 if(disposed||seq!==epoch||projectId!==props.projectId)return;policy.value=value;select.value=value.mode;emit('changed',value);
 if(typeof window!=='undefined'&&typeof CustomEvent!=='undefined')window.dispatchEvent(new CustomEvent('oviraptor:access-policy-changed',{detail:{projectId}}));
 }catch(e){if(seq===epoch)error.value=tr('权限模式未保存，请刷新后重试','Permission mode was not saved. Refresh and retry.')+` (${String((e as Error).message||e)})`;}
 finally{if(seq===epoch)saving.value=false;}
}
function externalChange(event:Event){if((event as CustomEvent).detail?.projectId===props.projectId&&!saving.value)void load();}
if(typeof window!=='undefined')window.addEventListener('oviraptor:access-policy-changed',externalChange);
watch(()=>[props.projectId,props.runtimeIdentity,props.client],()=>{saving.value=false;void load();},{immediate:true});
onBeforeUnmount(()=>{disposed=true;++epoch;if(typeof window!=='undefined')window.removeEventListener('oviraptor:access-policy-changed',externalChange);});
</script>
<template>
 <div class="agent-permission-mode" :class="{compact}" data-testid="agent-permission-mode">
  <label><ShieldCheck :size="14"/><span v-if="!compact">{{tr('Agent 权限模式','Agent permission mode')}}</span><select :value="policy?.mode||'ask'" :disabled="disabled||loading||saving||!policy" :aria-label="tr('Agent 权限模式','Agent permission mode')" @change="change"><option value="ask">{{tr('每次询问','Ask each time')}}</option><option value="full">{{tr('完全放开','Full access')}}</option></select><small v-if="saving">{{tr('保存中…','Saving…')}}</small></label>
  <p v-if="!compact" class="mode-summary">{{policy?.mode==='full'?tr('自动允许当前已启用应用、会话文件读写及隔离脚本执行。','Allows enabled apps, conversation file access and isolated script execution.'):tr('未授权操作逐次确认；已保存的应用授权继续生效。','Confirms ungranted operations; saved app grants remain active.')}}</p>
 <details v-if="!compact" class="mode-details"><summary>{{tr('作用范围与模式说明','Scope and mode details')}}</summary><p class="mode-help">{{tr('对本工作空间的所有对话生效，配置中心与聊天同步，重启后保留。每次询问：未授权操作逐次确认，已保存的应用授权继续生效。完全放开：自动允许当前已启用应用及会话文件读写及隔离脚本执行；仍遵守目标范围、积分和工具本身的限制。','Shared by Configuration and every chat in this workspace, and retained after restart. Ask confirms ungranted operations and respects saved app grants. Full access permits currently enabled apps and conversation files and isolated scripts; target scopes, costs and tool limits still apply.')}}</p></details>
  <p v-if="policy?.needsReview" class="mode-help">{{tr('模型接收方已变化，请重新选择权限模式。','The model recipient changed. Select the mode again.')}}</p>
  <p v-if="error" class="mode-error" role="alert">{{error}} <button v-refresh-feedback="{scope:projectId,busy:loading,error}" type="button" @click="load">{{tr('刷新','Refresh')}}</button></p>
 </div>
</template>
<style scoped>
.agent-permission-mode{min-width:0;font-size:12px}.agent-permission-mode>label{display:flex;align-items:center;gap:7px;flex-wrap:wrap}.agent-permission-mode select{width:auto;min-width:105px;max-width:100%;min-height:30px;padding:4px 24px 4px 8px;font:inherit;border:1px solid var(--line,#ddd);border-radius:6px;background:var(--app-surface,#fff);color:inherit}.agent-permission-mode.compact select{border-color:transparent;background:transparent;font-size:11px;min-height:25px}.mode-help,.mode-error{font-size:11px;line-height:1.6;margin:7px 0}.mode-help{color:var(--muted,#777)}.mode-error{color:#b04432}.mode-error button{font:inherit;background:transparent;border:1px solid var(--line);border-radius:5px;color:inherit}.compact .mode-error{max-width:360px}
.agent-permission-mode:not(.compact){font:var(--text-body)/1.6 var(--font-sans);color:var(--ui-ink)}.agent-permission-mode:not(.compact)>label{gap:8px 10px}.agent-permission-mode:not(.compact)>label>span{font-size:var(--text-meta);font-weight:600}.agent-permission-mode:not(.compact) select{box-sizing:border-box;height:38px;min-width:160px;padding:0 30px 0 10px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface);color:var(--ui-ink);font:var(--text-body)/1.5 var(--font-sans)}.agent-permission-mode:not(.compact) select:focus-visible,.mode-details>summary:focus-visible{outline:2px solid var(--ui-accent);outline-offset:3px}.mode-summary{margin:9px 0 4px;color:var(--ui-muted);font-size:var(--text-meta);line-height:1.65;overflow-wrap:anywhere}.mode-details{color:var(--ui-muted);font-size:var(--text-meta)}.mode-details>summary{cursor:pointer;line-height:1.8}.mode-details .mode-help{margin:8px 0 0;padding:10px 12px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised);font-size:var(--text-meta);line-height:1.7;overflow-wrap:anywhere}.agent-permission-mode:not(.compact)>.mode-help{font-size:var(--text-meta);color:color-mix(in srgb,var(--amber) 82%,var(--ui-ink))}.agent-permission-mode:not(.compact)>.mode-error{margin:10px 0 0;padding:10px 12px;border:1px solid color-mix(in srgb,var(--red) 35%,var(--ui-line));border-radius:var(--radius-md);color:color-mix(in srgb,var(--red) 82%,var(--ui-ink));font-size:var(--text-meta);overflow-wrap:anywhere;white-space:pre-wrap}.agent-permission-mode:not(.compact) .mode-error button{padding:3px 8px;background:var(--ui-surface)}
</style>
