<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { Eye, EyeOff } from '@lucide/vue';
type Settings = {revision:number;keyConfigured:boolean;maxRequests:number;maxResults:number;networkMode:string};
type Account = {type?:string;rest_free_point?:number;rest_equity_point?:number;once_export_quota?:number};
const loading=ref(true),busy=ref(false),error=ref(''),notice=ref(''),key=ref(''),visible=ref(false),clearKey=ref(false),loadFailed=ref(false);
const account=ref<Account>();
const settings=ref<Settings>({revision:0,keyConfigured:false,maxRequests:0,maxResults:100,networkMode:'direct'});
const baseline=ref('');
const snapshot=()=>JSON.stringify([settings.value.maxResults,settings.value.networkMode]);
const dirty=computed(()=>!!key.value.trim()||clearKey.value||snapshot()!==baseline.value);
const canVerify=computed(()=>!loading.value&&!busy.value&&!loadFailed.value&&!clearKey.value&&(!!key.value.trim()||settings.value.keyConfigured));
async function load(){loading.value=true;error.value='';try{const result=await invoke<Settings>('get_hunter_discovery_settings');settings.value={...settings.value,...result};baseline.value=snapshot();loadFailed.value=false;}catch(e){error.value=String(e);loadFailed.value=true;}finally{loading.value=false;}}
async function persist(){
 const result=await invoke<Settings>('save_hunter_discovery_settings',{input:{expectedRevision:settings.value.revision,key:key.value.trim()||null,clearKey:clearKey.value,maxRequests:0,maxResults:Number(settings.value.maxResults),networkMode:settings.value.networkMode}});
 settings.value={...settings.value,...result};baseline.value=snapshot();key.value='';clearKey.value=false;visible.value=false;account.value=undefined;
 notice.value=settings.value.keyConfigured?'API Key 与设置已保存到本机。':'已移除 API Key，其他设置已保存。';
}
async function save(){if(loading.value||busy.value||loadFailed.value)return;busy.value=true;error.value='';notice.value='';try{await persist();notice.value+=' 未验证连接，未查询资产。';}catch(e){error.value='保存失败，输入已保留：'+String(e);}finally{busy.value=false;}}
async function testConnection(){
 if(!canVerify.value)return;busy.value=true;error.value='';notice.value='';account.value=undefined;
 let saved=false;
 try{
  if(dirty.value){await persist();saved=true;}
  account.value=await invoke<Account>('test_hunter_discovery_api');notice.value=(saved?'API Key 已保存，':'已保存的 API Key ' )+'账户验证通过。未查询资产，未消耗查询积分。';
 }catch(e){error.value=(saved?'API Key 已保存；账户验证失败：':dirty.value?'保存失败，输入已保留：':'账户验证失败：')+String(e);}
 finally{busy.value=false;}
}
const quota=(value:number|undefined)=>value===undefined?'未报告':value===-1?'不限':value.toLocaleString();
onMounted(load);
</script>
<template>
<section class="hunter-settings" data-testid="hunter-account-settings">
<header class="hunter-heading"><div><h3>Hunter 账户</h3><p>保存 API Key 后验证账户；验证不会查询资产或消耗查询积分。</p></div><span class="hunter-key-state" :class="{'is-saved':settings.keyConfigured}">{{key.trim()?'新 Key 待保存':settings.keyConfigured?'已保存 API Key':'未配置 API Key'}}</span></header>
<p v-if="loading" role="status">正在读取 Hunter 设置…</p>
<fieldset :disabled="loading||busy||loadFailed">
<label class="field"><span>API Key</span><div class="secret-input"><input v-model="key" :type="visible?'text':'password'" :placeholder="settings.keyConfigured?'留空保留已保存的 Key，输入新 Key 可替换':'粘贴 Hunter API Key'" autocomplete="new-password" spellcheck="false"/><button type="button" class="button ghost compact" :aria-label="visible?'隐藏 Hunter Key':'显示 Hunter Key'" @click="visible=!visible"><EyeOff v-if="visible" :size="16"/><Eye v-else :size="16"/></button></div><small>{{key.trim()?'输入尚未保存，点击下方“保存并验证 API Key”。':'已保存的 Key 不回显；留空不会清除。'}}</small></label>
<div class="hunter-options"><label class="field"><span>连接方式</span><select v-model="settings.networkMode"><option value="direct">Hunter 独立直连（推荐）</option><option value="business">跟随业务网络 / 代理</option></select></label><label class="field"><span>每页返回条数</span><select v-model.number="settings.maxResults"><option :value="10">10</option><option :value="20">20</option><option :value="50">50</option><option :value="100">100</option></select></label></div>
<p class="helper">{{settings.networkMode==='direct'?'忽略应用与环境代理，使用系统网络。全局 VPN 仍可接管流量，请关闭 VPN 后验证。':'账户验证与分页查询均使用业务网络配置。全局 VPN 出口可能被 Hunter 拒绝。'}}</p>
<div class="hunter-settings-actions"><button type="button" class="button primary" :disabled="!canVerify" @click="testConnection">{{busy?'正在处理…':dirty?'保存并验证 API Key':'验证已保存的 API Key'}}</button><button type="button" class="button ghost" :disabled="!dirty" @click="save">仅保存，不验证连接</button></div>
<details v-if="settings.keyConfigured" class="hunter-remove"><summary>移除 API Key</summary><label class="check-field"><input v-model="clearKey" type="checkbox"/>下次保存时移除已保存的 Hunter Key</label></details>
</fieldset>
<p v-if="notice" class="hunter-notice" role="status">{{notice}}</p><div v-if="error" class="hunter-error" role="alert"><strong>{{error.startsWith('保存失败')?'设置尚未保存':'账户验证未通过'}}</strong><p>{{error}}</p><p v-if="error.includes('403')">Hunter 拒绝了当前请求。独立直连仍会受全局 VPN 接管，请关闭 VPN 后重新验证；403 不能证明 Key 错误。</p><button v-if="loadFailed" class="button ghost compact" @click="load">重新读取设置</button></div>
<dl v-if="account" class="hunter-account-summary"><div><dt>每日免费积分</dt><dd>{{quota(account.rest_free_point)}}</dd></div><div><dt>权益积分</dt><dd>{{quota(account.rest_equity_point)}}</dd></div><div><dt>单次导出上限</dt><dd>{{quota(account.once_export_quota)}}</dd></div></dl>
<p class="hunter-billing-note">资产查询按精确域名、证书指纹或 CNAME 自动翻页，可使用全部可用积分；每页和累计消耗在查询进度中展示。</p>
<p class="hunter-billing-note">连接与分页诊断保存在应用数据目录的 <code>diagnostics/hunter.jsonl</code>：包含查询条件、时间范围、页码、HTTP / Hunter 错误、耗时和积分回执；Key 与账户隐私脱敏。每 8 MiB 轮转，保留上一份日志；请求结果未知时不会自动重发。</p>
</section>
</template>
<style scoped>
.hunter-settings{margin-top:28px;padding-top:24px;border-top:1px solid var(--line);max-width:880px;font-size:13px;line-height:1.6;color:var(--app-ink)}
.hunter-heading{display:flex;align-items:flex-start;justify-content:space-between;gap:16px;margin-bottom:20px}.hunter-heading h3{font-size:17px;margin:0 0 5px}.hunter-heading p{font-size:12px;color:var(--muted);margin:0}.hunter-key-state{flex-shrink:0;padding:3px 9px;border-radius:6px;background:var(--app-sidebar);color:var(--muted);font-size:11px}.hunter-key-state.is-saved{color:#18734d;background:#eaf6ef}
.hunter-settings fieldset{border:0;padding:0;margin:0;min-width:0}.hunter-settings .field{margin:0;display:flex;flex-direction:column;gap:7px}.hunter-settings .field>span{font-size:12px;font-weight:550}.hunter-settings .field small,.hunter-settings .helper{font-size:12px;color:var(--muted)}.hunter-settings .helper{margin:10px 0 0}.hunter-settings input:not([type=checkbox]),.hunter-settings select{font-size:13px;min-height:38px;width:100%}.secret-input{display:flex;gap:8px;align-items:center}.secret-input input{flex:1;min-width:0}.secret-input button{width:38px;min-height:38px;flex-shrink:0;display:grid;place-items:center}.hunter-options{display:grid;grid-template-columns:minmax(0,2fr) minmax(120px,1fr);gap:16px;margin-top:20px}.hunter-settings-actions{display:flex;align-items:center;flex-wrap:wrap;gap:12px;margin-top:20px}.hunter-settings-actions .button{min-height:38px;font-size:12px;padding:8px 16px}.hunter-remove{margin-top:18px;font-size:12px;color:var(--muted)}.hunter-remove .check-field{display:flex;gap:8px;align-items:center;margin-top:10px}.hunter-notice{padding:10px 12px;border-radius:8px;background:#eaf6ef;color:#18734d;font-size:12px}.hunter-error{padding:12px 14px;background:var(--app-surface);border:1px solid #e6b6b6;border-radius:8px;margin-top:16px;font-size:12px;overflow-wrap:anywhere}.hunter-error strong{font-size:12px}.hunter-error p{margin:6px 0;color:#ae3535}.hunter-account-summary{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:16px;margin:18px 0;padding:16px;background:var(--app-sidebar);border-radius:8px}.hunter-account-summary dt{font-size:11px;color:var(--muted)}.hunter-account-summary dd{margin:4px 0 0;font-size:18px;font-weight:600}.hunter-billing-note{margin:20px 0 0;font-size:12px;color:var(--muted);line-height:1.7}
@media(max-width:650px){.hunter-heading{flex-direction:column;gap:8px}.hunter-options{grid-template-columns:1fr}.hunter-settings-actions{align-items:stretch;flex-direction:column}.hunter-account-summary{gap:10px;padding:12px}.hunter-account-summary dd{font-size:16px}}
</style>
