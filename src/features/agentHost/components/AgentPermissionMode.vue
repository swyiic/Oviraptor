<script setup lang="ts">
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
  <p v-if="!compact" class="mode-help">{{tr('对本工作空间的所有对话生效，配置中心与聊天同步，重启后保留。每次询问：未授权操作逐次确认，已保存的应用授权继续生效。完全放开：自动允许当前已启用应用及会话文件读写；仍遵守目标范围、积分和工具本身的限制。','Shared by Configuration and every chat in this workspace, and retained after restart. Ask confirms ungranted operations and respects saved app grants. Full access permits currently enabled apps and conversation files; target scopes, costs and tool limits still apply.')}}</p>
  <p v-if="policy?.needsReview" class="mode-help">{{tr('模型接收方已变化，请重新选择权限模式。','The model recipient changed. Select the mode again.')}}</p>
  <p v-if="error" class="mode-error" role="alert">{{error}} <button type="button" @click="load">{{tr('刷新','Refresh')}}</button></p>
 </div>
</template>
<style scoped>
.agent-permission-mode{min-width:0;font-size:12px}.agent-permission-mode>label{display:flex;align-items:center;gap:7px;flex-wrap:wrap}.agent-permission-mode select{width:auto;min-width:105px;max-width:100%;min-height:30px;padding:4px 24px 4px 8px;font:inherit;border:1px solid var(--line,#ddd);border-radius:6px;background:var(--app-surface,#fff);color:inherit}.agent-permission-mode.compact select{border-color:transparent;background:transparent;font-size:11px;min-height:25px}.mode-help,.mode-error{font-size:11px;line-height:1.6;margin:7px 0}.mode-help{color:var(--muted,#777)}.mode-error{color:#b04432}.mode-error button{font:inherit;background:transparent;border:1px solid var(--line);border-radius:5px;color:inherit}.compact .mode-error{max-width:360px}
</style>
