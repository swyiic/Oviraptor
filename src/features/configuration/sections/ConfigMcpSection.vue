<script setup lang="ts">
// MCP connectors (design: external-MCP decision connector, jev-studio first).
// jev-studio is a typed decision engine exposed over stdio MCP — not a chat
// model, so it never joins model connections. Its tools are surfaced to the
// agent as advisory `jev_*` decision helpers: they help the agent make
// disciplined typed decisions, same wall as memory/technique cards/the tool
// library — never an input to any auth/scope/settlement/capability gate.
// Disabled by default; any failure (missing binary, bad handshake, crash)
// degrades to "no extra tools" and the agent keeps running.
import { Brain, Check, CircleAlert, Loader2, Power, RefreshCw } from '@lucide/vue';
import { onMounted, ref } from 'vue';
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { useI18n } from '../../../i18n';
import { resolveAgentHostEndpoint } from '../../agentHost/api';
import { createAgentHostChatClient, type AgentHostChatClient, type McpJevStatus } from '../../agentHost/chatClient';

const { tr } = useI18n();
const client = ref<AgentHostChatClient | null>(null);
const status = ref<McpJevStatus | null>(null);
const unavailable = ref(false);
const loading = ref(false);
const saving = ref(false);
const error = ref('');
const advancedOpen = ref(false);

// apiKey is write-only: the server never returns the value, so the field
// always starts empty and is cleared again after a save. Leaving it blank
// keeps whatever key is already configured.
const form = ref({ enabled: false, apiKey: '', command: 'jev-studio', argsText: '' });

function applyStatus(next: McpJevStatus) {
  status.value = next;
  form.value = { enabled: next.enabled, apiKey: '', command: next.command || 'jev-studio', argsText: next.args.join(' ') };
}

async function load() {
  if (!client.value) return;
  loading.value = true; error.value = '';
  try { applyStatus(await client.value.mcpJevStatus()); }
  catch (e) { error.value = String((e as Error).message || e); }
  finally { loading.value = false; }
}

async function init() {
  const endpoint = await resolveAgentHostEndpoint();
  if (!endpoint) { unavailable.value = true; return; }
  client.value = createAgentHostChatClient(endpoint);
  await load();
}

async function save() {
  if (!client.value || saving.value) return;
  saving.value = true; error.value = '';
  try {
    const args = form.value.argsText.split(/\s+/).map(part => part.trim()).filter(Boolean);
    const key = form.value.apiKey.trim();
    applyStatus(await client.value.setMcpJev({ enabled: form.value.enabled, command: form.value.command.trim() || 'jev-studio', args, ...(key ? { apiKey: key } : {}) }));
  } catch (e) { error.value = String((e as Error).message || e); }
  finally { saving.value = false; }
}

function stateLabel(state: McpJevStatus['state']) {
  return state === 'ready' ? tr('已就绪', 'Ready')
    : state === 'starting' ? tr('启动中', 'Starting')
      : state === 'error' ? tr('出错', 'Error')
        : tr('已关闭', 'Disabled');
}

onMounted(init);
</script>
<template>
  <section class="panel config-domain-panel mcp-section">
    <p class="config-domain-description">{{ tr('接入外部 MCP server,把它的工具交给 agent 用。第一个:jev-studio(TypeSafe Jev 决策引擎),它的工具是建议性的决策帮手——帮 agent 在分叉点做有纪律的类型化决策,不授予任何权限、不改变 agent 能对谁做什么。默认关闭;启动失败会优雅降级为"没有这组额外工具",不影响其余功能。', 'Connect an external MCP server and hand its tools to the agent. First one: jev-studio (TypeSafe Jev\'s decision engine) — its tools are advisory decision helpers that keep the agent\'s choices disciplined and typed at a fork; they grant no authority and never change what the agent may act on. Off by default; any startup failure degrades gracefully to "no extra tools" without affecting anything else.') }}</p>
    <p v-if="unavailable" class="mcp-muted" role="status">{{ tr('agent 运行时未连接,暂时无法配置 MCP 连接器。', 'Agent runtime not connected; MCP connectors are unavailable right now.') }}</p>
    <template v-else>
      <article class="panel mcp-card" data-testid="mcp-jev-panel">
        <header>
          <strong><Brain :size="16" /> jev-studio</strong>
          <div class="mcp-head-actions">
            <span v-if="status" class="mcp-state" :data-state="status.state">
              <Loader2 v-if="status.state === 'starting'" :size="12" class="spinning" />
              <Check v-else-if="status.state === 'ready'" :size="12" />
              <CircleAlert v-else-if="status.state === 'error'" :size="12" />
              <Power v-else :size="12" />
              {{ stateLabel(status.state) }}
            </span>
            <button v-refresh-feedback class="mcp-icon" type="button" :disabled="loading" :title="tr('刷新状态', 'Refresh status')" :aria-label="tr('刷新 jev 连接器状态', 'Refresh jev connector status')" @click="load()"><RefreshCw :size="14" :class="{ spinning: loading }" /></button>
          </div>
        </header>
        <p class="mcp-desc">{{ tr('jev 是一个决策引擎(pip 装的 jev-studio),通过 MCP stdio 把它的决策工具(如 Choice/Score)交给 agent,前缀 jev_。', 'jev is a decision engine (pip-installed jev-studio) that exposes its decision tools (e.g. Choice/Score) to the agent over MCP stdio, prefixed jev_.') }}</p>
        <form class="mcp-form" @submit.prevent="save">
          <label class="mcp-toggle"><input v-model="form.enabled" type="checkbox" />{{ tr('启用 jev 决策连接器', 'Enable the jev decision connector') }}</label>
          <label class="mcp-field">
            {{ tr('API Key（TypeSafe）', 'API Key (TypeSafe)') }}
            <span class="mcp-key-state" :data-set="!!status?.hasApiKey">{{ status?.hasApiKey ? tr('已设置', 'Set') : tr('未设置', 'Not set') }}</span>
            <input v-model="form.apiKey" type="password" autocomplete="off" :placeholder="status?.hasApiKey ? tr('留空则不变', 'Leave blank to keep the current key') : tr('粘贴 TypeSafe 控制台的 key', 'Paste the key from the TypeSafe console')" />
          </label>
          <p class="mcp-help">{{ tr('填 TypeSafe 控制台的 key，会以 TYPESAFE_API_KEY 注入 jev-studio 进程', 'Enter the key from the TypeSafe console; it is injected into the jev-studio process as TYPESAFE_API_KEY') }}</p>
          <details class="mcp-advanced" :open="advancedOpen" @toggle="advancedOpen=($event.target as HTMLDetailsElement).open">
            <summary>{{ tr('高级', 'Advanced') }}</summary>
            <div class="mcp-row">
              <label class="mcp-field">{{ tr('命令', 'Command') }}<input v-model="form.command" placeholder="jev-studio" /></label>
              <label class="mcp-field">{{ tr('参数（空格分隔，可选）', 'Args (space separated, optional)') }}<input v-model="form.argsText" :placeholder="tr('例如：--config ./jev.toml', 'e.g. --config ./jev.toml')" /></label>
            </div>
          </details>
          <button class="button primary compact" type="submit" :disabled="saving">{{ saving ? tr('保存中…', 'Saving…') : tr('保存', 'Save') }}</button>
        </form>
        <p v-if="status?.state === 'error' && status.error" class="mcp-error" role="alert">{{ status.error }}</p>
        <p v-if="error" class="mcp-error" role="alert">{{ error }}</p>
        <div v-if="status?.state === 'ready' && status.tools.length" class="mcp-tools">
          <strong>{{ tr('可用决策工具', 'Available decision tools') }}</strong>
          <ul>
            <li v-for="tool in status.tools" :key="tool.name"><code>{{ tool.name }}</code><span>{{ tool.title }}</span><small v-if="tool.description">{{ tool.description }}</small></li>
          </ul>
        </div>
      </article>
    </template>
  </section>
</template>
<style scoped>
.mcp-section{display:flex;flex-direction:column;font-size:var(--text-body);color:var(--ui-ink)}
.mcp-section .config-domain-description{margin:0 0 16px}
.mcp-muted{font-size:var(--text-meta);color:var(--ui-muted);margin:8px 0}
.mcp-card{padding:18px 20px;margin:0;min-width:0;border:1px solid var(--ui-line);background:var(--ui-surface);border-radius:var(--radius-lg);box-shadow:none}
.mcp-card header{display:flex;justify-content:space-between;align-items:center;gap:12px}
.mcp-card header strong{display:inline-flex;align-items:center;gap:8px;font-size:var(--text-heading);font-weight:600}
.mcp-head-actions{display:flex;align-items:center;gap:8px}
.mcp-state{display:inline-flex;align-items:center;gap:5px;font-size:var(--text-micro);font-weight:600;border:1px solid var(--ui-line);border-radius:var(--radius-xs);padding:3px 8px;color:var(--ui-muted);background:var(--ui-surface-raised)}
.mcp-state[data-state="ready"]{color:var(--green);border-color:var(--green);background:color-mix(in srgb,var(--green) 9%,var(--ui-surface))}
.mcp-state[data-state="error"]{color:var(--red);border-color:var(--red)}
.mcp-state[data-state="starting"]{color:var(--blue);border-color:var(--blue)}
.mcp-icon{display:inline-flex;align-items:center;justify-content:center;width:32px;height:32px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:transparent;padding:0;color:inherit;flex-shrink:0;cursor:pointer}
.mcp-icon:hover:not(:disabled){background:var(--ui-surface-raised)}
.mcp-desc{font-size:var(--text-meta);color:var(--ui-muted);margin:10px 0 16px;line-height:1.6}
.mcp-form{display:flex;flex-direction:column;gap:14px}
.mcp-toggle{display:flex;align-items:center;gap:9px;font-size:var(--text-body)}
.mcp-toggle input{width:15px;height:15px;accent-color:var(--blue)}
.mcp-row{display:flex;gap:16px;flex-wrap:wrap}
.mcp-field{display:flex;flex-direction:column;gap:6px;font-size:var(--text-meta);font-weight:500;color:var(--ui-muted);flex:1;min-width:160px}
.mcp-field input,.mcp-toggle input[type="text"]{box-sizing:border-box;height:38px;min-height:38px;font-size:var(--text-body);padding:0 10px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface);color:var(--ui-ink);width:100%;font-family:inherit}
.mcp-field input:focus{border-color:var(--blue);outline:2px solid color-mix(in srgb,var(--blue) 18%,transparent);outline-offset:1px}
.mcp-key-state{margin-left:6px;font-size:var(--text-micro);font-weight:600;color:var(--ui-muted)}
.mcp-key-state[data-set="true"]{color:var(--green)}
.mcp-help{margin:0;font-size:var(--text-meta);color:var(--ui-muted);line-height:1.6}
.mcp-advanced{font-size:var(--text-meta);border-top:1px solid var(--ui-line);padding-top:12px}
.mcp-advanced>summary{cursor:pointer;color:var(--ui-muted);font-weight:500;width:fit-content}
.mcp-advanced>.mcp-row{margin-top:12px}
.mcp-error{color:var(--red);font-size:var(--text-meta);overflow-wrap:anywhere;margin:4px 0 0}
.mcp-tools{margin-top:16px;padding-top:16px;border-top:1px solid var(--ui-line);font-size:var(--text-meta)}
.mcp-tools strong{font-size:var(--text-micro);color:var(--ui-muted);font-weight:600;text-transform:uppercase;letter-spacing:.02em}
.mcp-tools ul{list-style:none;margin:9px 0 0;padding:0;display:flex;flex-direction:column;gap:8px}
.mcp-tools li{display:flex;flex-direction:column;gap:3px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised);padding:9px 11px}
.mcp-tools code{font-size:var(--text-micro);color:var(--blue)}
.mcp-tools span{font-size:var(--text-body)}
.mcp-tools small{color:var(--ui-muted);line-height:1.5}
.spinning{animation:mcp-spin .8s linear infinite}
@keyframes mcp-spin{to{transform:rotate(360deg)}}
@media(max-width:520px){.mcp-row{flex-direction:column}}
</style>
