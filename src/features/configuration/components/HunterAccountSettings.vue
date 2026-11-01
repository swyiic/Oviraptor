<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { Eye, EyeOff } from '@lucide/vue';
import { useI18n } from '../../../i18n';
type Settings = {revision:number;keyConfigured:boolean;maxRequests:number;maxResults:number;networkMode:string};
type Account = {type?:string;rest_free_point?:number;rest_equity_point?:number;once_export_quota?:number};
const loading=ref(true),busy=ref(false),error=ref(''),notice=ref(''),key=ref(''),visible=ref(false),clearKey=ref(false),loadFailed=ref(false);
const { tr } = useI18n();
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
 notice.value=settings.value.keyConfigured?tr('API Key 与设置已保存到本机。','API key and settings saved on this device.'):tr('已移除 API Key，其他设置已保存。','API key removed; other settings saved.');
}
async function save(){if(loading.value||busy.value||loadFailed.value)return;busy.value=true;error.value='';notice.value='';try{await persist();notice.value+=tr(' 未验证连接，未查询资产。',' Connection not verified; no assets were queried.');}catch(e){error.value=tr(`保存失败，输入已保留：${String(e)}`,`Save failed; your input was kept: ${String(e)}`);}finally{busy.value=false;}}
async function testConnection(){
 if(!canVerify.value)return;busy.value=true;error.value='';notice.value='';account.value=undefined;
 let saved=false;
 try{
  if(dirty.value){await persist();saved=true;}
  account.value=await invoke<Account>('test_hunter_discovery_api');notice.value=saved?tr('API Key 已保存，账户验证通过。未查询资产，未消耗查询积分。','API key saved and account verified. No assets were queried and no query credits were used.'):tr('已保存的 API Key 账户验证通过。未查询资产，未消耗查询积分。','Saved API key verified. No assets were queried and no query credits were used.');
 }catch(e){error.value=saved?tr(`API Key 已保存；账户验证失败：${String(e)}`,`API key saved; account verification failed: ${String(e)}`):dirty.value?tr(`保存失败，输入已保留：${String(e)}`,`Save failed; your input was kept: ${String(e)}`):tr(`账户验证失败：${String(e)}`,`Account verification failed: ${String(e)}`);}
 finally{busy.value=false;}
}
const quota=(value:number|undefined)=>value===undefined?tr('未报告','Not reported'):value===-1?tr('不限','Unlimited'):value.toLocaleString();
onMounted(load);
</script>
<template>
<section class="hunter-settings" data-testid="hunter-account-settings">
  <header class="hunter-heading">
    <div><h3>{{tr('Hunter 账户','Hunter account')}}</h3><p>{{tr('配置查询凭据。账户验证不查询资产，不消耗查询积分。','Configure query credentials. Account verification does not query assets or use query credits.')}}</p></div>
    <span class="hunter-key-state" :class="{'is-saved':settings.keyConfigured&&!key.trim()}">{{key.trim()?tr('新 Key 待保存','New key not saved'):settings.keyConfigured?tr('已配置','Configured'):tr('未配置','Not configured')}}</span>
  </header>
  <p v-if="loading" class="hunter-loading" role="status">{{tr('正在读取 Hunter 设置…','Loading Hunter settings…')}}</p>
  <fieldset :disabled="loading||busy||loadFailed" :aria-label="tr('Hunter 账户设置','Hunter account settings')">
    <div class="hunter-fields">
      <div class="field hunter-key-field">
        <label for="hunter-api-key">API Key</label>
        <div class="secret-input">
          <input id="hunter-api-key" v-model="key" :type="visible?'text':'password'" :placeholder="settings.keyConfigured?tr('留空保留已保存的 Key','Leave blank to keep the saved key'):tr('粘贴 Hunter API Key','Paste your Hunter API key')" aria-describedby="hunter-key-help" autocomplete="new-password" spellcheck="false"/>
          <button type="button" class="hunter-visibility" :aria-label="visible?tr('隐藏 Hunter Key','Hide Hunter key'):tr('显示 Hunter Key','Show Hunter key')" :aria-pressed="visible" @click="visible=!visible"><EyeOff v-if="visible" :size="16"/><Eye v-else :size="16"/></button>
        </div>
        <small id="hunter-key-help">{{key.trim()?tr('新 Key 尚未保存，保存后替换原值。','The new key is not saved yet; saving replaces the current key.'):tr('已保存的 Key 不回显；留空不会清除。','The saved key is not shown; leaving this blank keeps it.')}}</small>
      </div>
      <label class="field hunter-network-field"><span>{{tr('连接方式','Connection')}}</span><select v-model="settings.networkMode"><option value="direct">{{tr('独立直连（推荐）','Direct connection (recommended)')}}</option><option value="business">{{tr('跟随业务网络 / 代理','Use business network / proxy')}}</option></select></label>
      <label class="field hunter-page-field"><span>{{tr('每页返回','Results per page')}}</span><select v-model.number="settings.maxResults"><option :value="10">{{tr('10 条','10 results')}}</option><option :value="20">{{tr('20 条','20 results')}}</option><option :value="50">{{tr('50 条','50 results')}}</option><option :value="100">{{tr('100 条','100 results')}}</option></select></label>
    </div>
    <div class="hunter-settings-footer">
      <div class="hunter-settings-actions"><button type="button" class="button primary" :disabled="!canVerify" @click="testConnection">{{busy?tr('正在处理…','Working…'):dirty?tr('保存并验证','Save and verify'):tr('验证账户','Verify account')}}</button><button type="button" class="button ghost" :disabled="!dirty" @click="save">{{tr('仅保存','Save only')}}</button></div>
      <details v-if="settings.keyConfigured" class="hunter-remove"><summary>{{tr('移除 API Key','Remove API key')}}</summary><label class="check-field"><input v-model="clearKey" type="checkbox"/>{{tr('下次保存时移除已保存的 Hunter Key','Remove the saved Hunter key on next save')}}</label></details>
    </div>
  </fieldset>
  <p v-if="notice" class="hunter-notice" role="status">{{notice}}</p>
  <div v-if="error" class="hunter-error" role="alert"><strong>{{error.startsWith(tr('保存失败','Save failed'))?tr('设置尚未保存','Settings not saved'):tr('账户验证未通过','Account verification failed')}}</strong><p>{{error}}</p><p v-if="error.includes('403')">{{tr('Hunter 拒绝了当前请求。独立直连仍会受全局 VPN 接管，请关闭 VPN 后重新验证；403 不能证明 Key 错误。','Hunter rejected the request. A global VPN can still take over direct connections; turn off the VPN and verify again. A 403 does not prove the key is wrong.')}}</p><button v-if="loadFailed" class="button ghost compact" @click="load">{{tr('重新读取设置','Reload settings')}}</button></div>
  <dl v-if="account" class="hunter-account-summary"><div><dt>{{tr('每日免费积分','Daily free credits')}}</dt><dd>{{quota(account.rest_free_point)}}</dd></div><div><dt>{{tr('权益积分','Entitlement credits')}}</dt><dd>{{quota(account.rest_equity_point)}}</dd></div><div><dt>{{tr('单次导出上限','Per-export limit')}}</dt><dd>{{quota(account.once_export_quota)}}</dd></div></dl>
  <div class="hunter-support">
    <p class="hunter-billing-note"><span>{{tr('查询计费','Query billing')}}</span>{{tr('资产查询自动翻页，可能使用全部可用积分。','Asset queries page automatically and may use all available credits.')}}</p>
    <details class="hunter-details">
      <summary>{{tr('连接、分页与诊断说明','Connection, paging, and diagnostics')}}</summary>
      <div class="hunter-details-body">
        <p><strong>{{tr('连接方式','Connection')}}</strong>{{settings.networkMode==='direct'?tr('忽略应用与环境代理，使用系统网络。全局 VPN 仍可接管流量，请关闭 VPN 后验证。','Ignores app and environment proxies and uses the system network. A global VPN can still take over traffic; turn it off before verifying.'):tr('账户验证与分页查询均使用业务网络配置。全局 VPN 出口可能被 Hunter 拒绝。','Account verification and paged queries both use the business network settings. Hunter may reject global VPN exit addresses.')}}</p>
        <p><strong>{{tr('分页与积分','Paging and credits')}}</strong>{{tr('按精确域名、证书指纹或 CNAME 自动翻页；每页和累计消耗在查询进度中展示。','Pages automatically by exact domain, certificate fingerprint, or CNAME; per-page and total usage appear in query progress.')}}</p>
        <p><strong>{{tr('诊断日志','Diagnostic log')}}</strong>{{tr('保存在应用数据目录的','Saved in the app data directory at')}} <code>diagnostics/hunter.jsonl</code>{{tr('，包含查询条件、时间范围、页码、HTTP / Hunter 错误、耗时和积分回执；Key 与账户隐私脱敏。每 8 MiB 轮转并保留上一份；请求结果未知时不会自动重发。','. It records query conditions, time ranges, page numbers, HTTP / Hunter errors, durations, and credit receipts; keys and private account details are redacted. The log rotates every 8 MiB and keeps the previous file; requests with unknown results are not retried automatically.')}}</p>
      </div>
    </details>
  </div>
</section>
</template>
<style scoped>
.hunter-settings{container-type:inline-size;min-width:0;margin-top:20px;padding-top:20px;border-top:1px solid var(--ui-line);color:var(--ui-ink);font:var(--text-body)/1.6 var(--font-sans)}
.hunter-heading{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:16px}.hunter-heading>div{min-width:0}.hunter-heading h3{margin:0;font-size:var(--text-heading);font-weight:600;line-height:1.5}.hunter-heading p{margin:4px 0 0;color:var(--ui-muted);font-size:var(--text-meta)}
.hunter-key-state{flex-shrink:0;padding:3px 9px;border:1px solid var(--ui-line);border-radius:var(--radius-pill);background:var(--ui-surface-raised);color:var(--ui-muted);font-size:var(--text-meta);line-height:1.5}.hunter-key-state.is-saved{border-color:transparent;color:color-mix(in srgb,var(--green) 82%,var(--ui-ink));background:color-mix(in srgb,var(--green) 9%,var(--ui-surface))}
.hunter-loading{margin:0 0 12px;color:var(--ui-muted);font-size:var(--text-meta)}.hunter-settings fieldset{min-width:0;margin:0;padding:0;border:0}.hunter-fields{display:grid;grid-template-columns:minmax(0,2fr) minmax(180px,1fr) minmax(100px,.45fr);align-items:start;gap:14px}
.hunter-settings .field{display:flex;flex-direction:column;min-width:0;margin:0;gap:6px}.hunter-settings .field>span,.hunter-settings .field>label{color:var(--ui-muted);font-size:var(--text-meta);font-weight:600;line-height:1.4}.hunter-settings .field small{color:var(--ui-muted);font-size:var(--text-meta);line-height:1.5}
.hunter-settings input:not([type=checkbox]),.hunter-settings select{box-sizing:border-box;width:100%;min-width:0;height:38px;padding:0 10px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface);color:var(--ui-ink);font:var(--text-body)/1.5 var(--font-sans)}
.hunter-settings select:focus-visible{outline:2px solid var(--ui-accent);outline-offset:2px}.secret-input{display:flex;min-width:0;height:38px;box-sizing:border-box;align-items:center;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface)}.hunter-settings .secret-input input{flex:1;height:36px;border:0;background:transparent;box-shadow:none;outline:0}.secret-input:focus-within{border-color:var(--ui-accent);box-shadow:0 0 0 3px color-mix(in srgb,var(--ui-accent) 12%,transparent)}
.hunter-settings .secret-input input:focus{border:0;background:transparent;box-shadow:none;outline:0}
.hunter-visibility{display:grid;place-items:center;flex:0 0 36px;align-self:stretch;margin:3px 3px 3px 0;padding:0;border:0;border-radius:var(--radius-sm);background:transparent;color:var(--ui-muted);cursor:pointer}.hunter-visibility:hover:not(:disabled){background:var(--ui-surface-raised);color:var(--ui-ink)}.hunter-visibility:focus-visible{outline:2px solid var(--ui-accent);outline-offset:-2px}
.hunter-settings-footer{display:flex;flex-wrap:wrap;align-items:center;justify-content:space-between;gap:12px;margin-top:16px}.hunter-settings-actions{display:flex;flex-wrap:wrap;align-items:center;gap:8px}.hunter-settings-actions .button{min-height:36px;padding:8px 14px;font-size:var(--text-body)}
.hunter-remove{color:var(--ui-muted);font-size:var(--text-meta);max-width:100%}.hunter-remove summary{padding:6px 0;cursor:pointer}.hunter-remove[open]{flex-basis:100%}.hunter-remove .check-field{display:flex;align-items:center;gap:8px;margin:4px 0 0;padding:8px 10px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised);font-size:var(--text-meta)}.hunter-remove input{flex-shrink:0;margin:0}
.hunter-notice{margin:14px 0 0;padding:9px 12px;border-radius:var(--radius-md);background:color-mix(in srgb,var(--green) 9%,var(--ui-surface));color:color-mix(in srgb,var(--green) 82%,var(--ui-ink));font-size:var(--text-meta);overflow-wrap:anywhere}.hunter-error{margin-top:14px;padding:12px;border:1px solid color-mix(in srgb,var(--red) 38%,var(--ui-line));border-radius:var(--radius-md);background:var(--ui-surface);font-size:var(--text-body);overflow-wrap:anywhere}.hunter-error strong{font-size:var(--text-body)}.hunter-error p{margin:6px 0;color:color-mix(in srgb,var(--red) 82%,var(--ui-ink))}
.hunter-account-summary{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:12px;margin:16px 0 0;padding:12px 14px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised)}.hunter-account-summary dt{color:var(--ui-muted);font-size:var(--text-meta)}.hunter-account-summary dd{margin:3px 0 0;color:var(--ui-ink);font:600 var(--text-heading)/1.5 var(--font-mono);overflow-wrap:anywhere}
.hunter-support{display:grid;gap:8px;margin-top:16px;padding-top:12px;border-top:1px solid var(--ui-line)}.hunter-billing-note{margin:0;color:var(--ui-muted);font-size:var(--text-meta);line-height:1.6}.hunter-billing-note>span{margin-right:8px;color:var(--ui-copy);font-weight:600}.hunter-details{min-width:0;color:var(--ui-muted);font-size:var(--text-meta)}.hunter-details>summary{cursor:pointer;line-height:1.8}.hunter-details-body{display:grid;gap:8px;margin-top:10px;padding:12px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised)}.hunter-details-body p{margin:0;line-height:1.7;overflow-wrap:anywhere}.hunter-details-body strong{margin-right:8px;color:var(--ui-copy);font-weight:600}.hunter-details-body code{font:var(--text-meta)/1.6 var(--font-mono);overflow-wrap:anywhere}
@container (max-width:800px){.hunter-fields{grid-template-columns:minmax(0,1fr) minmax(110px,.45fr)}.hunter-key-field{grid-column:1/-1}}
@container (max-width:450px){.hunter-heading{align-items:flex-start}.hunter-fields{grid-template-columns:minmax(0,1fr)}.hunter-key-field{grid-column:auto}.hunter-settings-footer{align-items:flex-start;flex-direction:column}.hunter-settings-actions{width:100%}.hunter-settings-actions .button{flex:1}.hunter-account-summary{gap:8px;padding:10px}}
</style>
