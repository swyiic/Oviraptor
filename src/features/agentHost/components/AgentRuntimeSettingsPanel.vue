<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Bot, KeyRound, RefreshCw } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import { openAgentHostSettingsClient } from "../api";
import type { AgentHostSettings, AgentRuntimeId, HostSettingsClient } from "../api";

const emit = defineEmits<{
  notify: [type: "success" | "error" | "info", text: string];
}>();

const { tr } = useI18n();

const loading = ref(true);
const savingRuntime = ref(false);
const savingProvider = ref(false);
const unavailable = ref(false);
const unavailableReason = ref("");
const error = ref("");
const notice = ref("");
const settings = ref<AgentHostSettings | null>(null);
const choice = ref<AgentRuntimeId>("none");
const modelDraft = ref("");
const apiKeyDraft = ref("");
const originLabel = ref("");

let client: HostSettingsClient | null = null;

const runtimeOptions = computed(() => [
  {
    id: "none" as const,
    label: tr("无 · 仅本地消息", "None · local messages only"),
    hint: tr("不启动 Agent runtime；对话仍可用。", "No Agent runtime; chat remains available."),
  },
  {
    id: "grok-acp" as const,
    label: tr("Grok CLI · ACP runtime", "Grok CLI · ACP runtime"),
    hint: tr(
      "官方 Grok CLI（ACP）。选择此项不会复制 API Key，也不会自动登录；首条消息可能需要单独认证。",
      "Official Grok CLI (ACP). Choosing this does not copy an API key or start login; the first message may require separate authentication.",
    ),
  },
  {
    id: "xai-api" as const,
    label: tr("xAI Grok API · API Key", "xAI Grok API · API key"),
    hint: tr(
      "使用本机保存的 xAI API Key（无公开第三方 OAuth）。需先保存 Key 再选此 runtime。",
      "Uses the API key stored on this host (no public third-party OAuth). Save a key before selecting this runtime.",
    ),
    disabled: !settings.value?.apiProvider?.apiKeyConfigured,
  },
]);

const providerStatus = computed(() => {
  const provider = settings.value?.apiProvider;
  if (!provider?.configured) {
    return tr("尚未配置 xAI provider", "xAI provider not configured");
  }
  const key = provider.apiKeyConfigured
    ? tr("Key 已配置（不回显）", "Key configured (never shown)")
    : tr("Key 未配置", "Key not configured");
  return tr(
    `已配置：${provider.model || "—"} @ ${provider.baseUrl || "—"} · ${key}`,
    `Configured: ${provider.model || "—"} @ ${provider.baseUrl || "—"} · ${key}`,
  );
});

const hostRuntimeLine = computed(() => {
  const runtime = settings.value?.runtime;
  if (!runtime) return "";
  return tr(
    `Host runtime：${runtime.label || runtime.id} · ${runtime.state || "unknown"}`,
    `Host runtime: ${runtime.label || runtime.id} · ${runtime.state || "unknown"}`,
  );
});

function notify(type: "success" | "error" | "info", text: string) {
  emit("notify", type, text);
}

async function copyOrigin() {
  const value = originLabel.value?.trim();
  if (!value) return;
  try {
    await navigator.clipboard.writeText(value);
    notice.value = tr("已复制 Host 端点", "Host endpoint copied");
  } catch {
    notice.value = value;
  }
}

async function load() {
  loading.value = true;
  error.value = "";
  notice.value = "";
  unavailable.value = false;
  unavailableReason.value = "";
  client = null;
  try {
    const opened = await openAgentHostSettingsClient();
    if (!opened) {
      unavailable.value = true;
      unavailableReason.value = tr(
        "Agent host sidecar 不可用：无法读取 origin/session。请确认桌面应用已启动 sidecar，或检查环境变量 OVIRAPTOR_HOST_ORIGIN / OVIRAPTOR_HOST_SESSION。配置无法在此保存。",
        "Agent host sidecar unavailable: origin/session could not be resolved. Ensure the desktop sidecar is running, or set OVIRAPTOR_HOST_ORIGIN / OVIRAPTOR_HOST_SESSION. Settings cannot be saved here.",
      );
      settings.value = null;
      return;
    }
    client = opened.client;
    originLabel.value = opened.endpoint.origin;
    const value = await client.getSettings();
    settings.value = value;
    choice.value = value.runtime.id;
    modelDraft.value = value.apiProvider?.model || "";
    apiKeyDraft.value = "";
  } catch (err) {
    error.value = String((err as Error)?.message || err);
    settings.value = null;
    unavailable.value = true;
    unavailableReason.value = tr(
      `无法连接 Agent host：${error.value}`,
      `Could not reach Agent host: ${error.value}`,
    );
  } finally {
    loading.value = false;
  }
}

async function saveRuntime() {
  if (!client || !settings.value || savingRuntime.value) return;
  savingRuntime.value = true;
  error.value = "";
  notice.value = "";
  try {
    const value = await client.setRuntime(choice.value);
    settings.value = value;
    choice.value = value.runtime.id;
    notice.value = tr(
      "Runtime 偏好已保存。未发起登录、模型请求或消息重发。",
      "Runtime preference saved. No login, model request, or message resend was started.",
    );
    notify("success", notice.value);
  } catch (err) {
    error.value = String((err as Error)?.message || err);
    notify("error", error.value);
  } finally {
    savingRuntime.value = false;
  }
}

async function saveProvider() {
  if (!client || savingProvider.value) return;
  savingProvider.value = true;
  error.value = "";
  notice.value = "";
  try {
    const value = await client.setProvider({
      model: modelDraft.value.trim() || undefined,
      apiKey: apiKeyDraft.value.trim() || undefined,
    });
    settings.value = value;
    choice.value = value.runtime.id;
    modelDraft.value = value.apiProvider?.model || modelDraft.value;
    apiKeyDraft.value = "";
    notice.value = tr(
      "xAI provider 已保存。API Key 仅存于本机 host（0600），不会再次显示。",
      "xAI provider saved. The API key is stored only on this host (0600) and is never shown again.",
    );
    notify("success", notice.value);
  } catch (err) {
    error.value = String((err as Error)?.message || err);
    notify("error", error.value);
  } finally {
    savingProvider.value = false;
  }
}

onMounted(() => {
  void load();
});
</script>

<template>
  <section class="panel agent-runtime-settings" data-testid="agent-runtime-settings">
    <div class="panel-heading">
      <div>
        <span class="eyebrow">AGENT RUNTIME / xAI GROK</span>
        <h3>{{ tr("Agent runtime / xAI Grok", "Agent runtime / xAI Grok") }}</h3>
        <p>
          {{
            tr(
              "在桌面配置中心直接管理 agent-host 的 runtime 与 xAI API Key，无需另开浏览器。Runtime 选择与模型凭据彼此独立。",
              "Manage the agent-host runtime and xAI API key in the desktop config center—no separate browser. Runtime choice and model credentials are independent.",
            )
          }}
        </p>
      </div>
      <div class="hero-actions">
        <button
          class="button ghost compact"
          type="button"
          :disabled="loading || savingRuntime || savingProvider"
          @click="load"
        >
          <RefreshCw :size="14" :class="{ spinning: loading }" />
          {{ loading ? tr("读取中…", "Loading…") : tr("刷新", "Refresh") }}
        </button>
      </div>
    </div>

    <div v-if="unavailable" class="environment-preparation-warning" role="alert" data-testid="agent-runtime-unavailable">
      <strong>{{ tr("Sidecar 不可用 / Load failed", "Sidecar unavailable / Load failed") }}</strong>
      <p>{{ unavailableReason }}</p>
      <p v-if="originLabel" class="muted small" data-testid="agent-runtime-failed-origin">
        {{ tr("Host 端点（可复制后排查）", "Host endpoint (copy for debugging)") }} ·
        <code>{{ originLabel }}</code>
        <button class="button ghost compact" type="button" data-testid="agent-runtime-copy-origin" @click="copyOrigin">
          {{ tr("复制", "Copy") }}
        </button>
      </p>
      <p class="muted small">{{ tr("可点上方「刷新」重试。", "Use Refresh above to retry.") }}</p>
    </div>

    <template v-else-if="!loading && settings">
      <p v-if="originLabel" class="muted small">
        {{ tr("Host 端点", "Host endpoint") }} · {{ originLabel }}
      </p>
      <p v-if="hostRuntimeLine" class="muted small">{{ hostRuntimeLine }}</p>

      <div class="agent-runtime-block">
        <header>
          <Bot :size="18" />
          <strong>{{ tr("Agent runtime", "Agent runtime") }}</strong>
        </header>
        <p class="muted">
          {{
            tr(
              "Runtime 负责 Agent 循环与工具请求。选择 Grok CLI 不会复制 provider API Key，也不会启动认证。",
              "The runtime runs the Agent loop and tool requests. Choosing Grok CLI does not copy a provider API key or start authentication.",
            )
          }}
        </p>
        <label class="field">
          <span>{{ tr("当前选择", "Current choice") }}</span>
          <select
            v-model="choice"
            data-testid="agent-runtime-choice"
            :disabled="loading || savingRuntime"
            :aria-label="tr('Agent runtime', 'Agent runtime')"
          >
            <option
              v-for="option in runtimeOptions"
              :key="option.id"
              :value="option.id"
              :disabled="option.disabled"
            >
              {{ option.label }}
            </option>
          </select>
        </label>
        <p class="helper">{{ runtimeOptions.find((item) => item.id === choice)?.hint }}</p>
        <button
          class="button primary compact"
          type="button"
          data-testid="agent-runtime-save"
          :disabled="loading || savingRuntime || choice === settings.runtime.id"
          @click="saveRuntime"
        >
          {{ savingRuntime ? tr("保存中…", "Saving…") : tr("保存 runtime 偏好", "Save runtime preference") }}
        </button>
      </div>

      <div class="agent-runtime-block">
        <header>
          <KeyRound :size="18" />
          <strong>{{ tr("xAI Grok API", "xAI Grok API") }}</strong>
        </header>
        <p class="muted">
          {{
            tr(
              "xAI 对第三方应用提供 API Key 认证（无公开 OAuth 客户端流程）。Key 仅存本机 host，设置接口永不返回明文。",
              "xAI authenticates third-party apps with API keys (no public OAuth client flow). The key stays on this host; settings never return it.",
            )
          }}
        </p>
        <p class="muted small" data-testid="agent-provider-status">{{ providerStatus }}</p>
        <div class="agent-runtime-grid">
          <label class="field">
            <span>{{ tr("模型", "Model") }}</span>
            <input
              v-model="modelDraft"
              data-testid="agent-xai-model"
              placeholder="grok-4.7"
              autocomplete="off"
              :aria-label="tr('xAI 模型', 'xAI model')"
            />
          </label>
          <label class="field">
            <span>{{ tr("API Key", "API Key") }}</span>
            <input
              v-model="apiKeyDraft"
              data-testid="agent-xai-key"
              type="password"
              autocomplete="off"
              :placeholder="
                settings.apiProvider?.apiKeyConfigured
                  ? tr('（保持不变）', '(unchanged)')
                  : 'xai-…'
              "
              :aria-label="tr('xAI API Key', 'xAI API key')"
            />
          </label>
        </div>
        <button
          class="button secondary compact"
          type="button"
          data-testid="agent-provider-save"
          :disabled="loading || savingProvider || (!modelDraft.trim() && !apiKeyDraft.trim() && !settings.apiProvider?.apiKeyConfigured)"
          @click="saveProvider"
        >
          {{ savingProvider ? tr("保存中…", "Saving…") : tr("保存 xAI provider", "Save xAI provider") }}
        </button>
      </div>
    </template>

    <div v-else-if="loading" class="empty-state small">
      {{ tr("正在读取 Agent host 设置…", "Reading Agent host settings…") }}
    </div>

    <p v-if="error" class="host-action-error" role="alert">{{ error }}</p>
    <p v-if="notice" class="muted" role="status">{{ notice }}</p>
  </section>
</template>

<style scoped>
.agent-runtime-settings {
  display: grid;
  gap: 1rem;
}
.agent-runtime-block {
  display: grid;
  gap: 0.65rem;
  padding: 0.85rem 1rem;
  border: 1px solid color-mix(in srgb, var(--border, #334) 80%, transparent);
  border-radius: 12px;
  background: color-mix(in srgb, var(--panel, transparent) 92%, transparent);
}
.agent-runtime-block header {
  display: flex;
  align-items: center;
  gap: 0.45rem;
}
.agent-runtime-grid {
  display: grid;
  gap: 0.75rem;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
}
.muted {
  opacity: 0.82;
}
.muted.small,
.helper {
  font-size: 0.86rem;
  opacity: 0.78;
}
.field {
  display: grid;
  gap: 0.35rem;
}
.field span {
  font-size: 0.82rem;
  font-weight: 600;
}
.field input,
.field select {
  width: 100%;
}
</style>
