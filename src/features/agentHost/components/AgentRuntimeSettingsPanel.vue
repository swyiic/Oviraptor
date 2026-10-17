<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Bot, Copy, KeyRound, RefreshCw } from "@lucide/vue";
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

const keyConfiguredLabel = computed(() => {
  if (settings.value?.apiProvider?.apiKeyConfigured) {
    return tr("已配置", "Configured");
  }
  return tr("未配置", "Not configured");
});

const selectedHint = computed(
  () => runtimeOptions.value.find((item) => item.id === choice.value)?.hint || "",
);

const busy = computed(() => loading.value || savingRuntime.value || savingProvider.value);

function notify(type: "success" | "error" | "info", text: string) {
  emit("notify", type, text);
}

async function copyOrigin() {
  const value = originLabel.value?.trim();
  if (!value) return;
  try {
    await navigator.clipboard.writeText(value);
    notice.value = tr("已复制 Host 端点", "Host endpoint copied");
    notify("info", notice.value);
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
          :disabled="busy"
          @click="load"
        >
          <RefreshCw :size="14" :class="{ spinning: loading }" />
          {{ loading ? tr("读取中…", "Loading…") : tr("刷新", "Refresh") }}
        </button>
      </div>
    </div>

    <div
      v-if="unavailable"
      class="agent-runtime-alert"
      role="alert"
      data-testid="agent-runtime-unavailable"
    >
      <strong>{{ tr("Sidecar 不可用", "Sidecar unavailable") }}</strong>
      <p>{{ unavailableReason }}</p>
      <div v-if="originLabel" class="agent-runtime-alert-row" data-testid="agent-runtime-failed-origin">
        <span>{{ tr("Host 端点", "Host endpoint") }}</span>
        <code>{{ originLabel }}</code>
        <button
          class="button ghost compact"
          type="button"
          data-testid="agent-runtime-copy-origin"
          @click="copyOrigin"
        >
          <Copy :size="13" />
          {{ tr("复制", "Copy") }}
        </button>
      </div>
      <p class="helper">{{ tr("可点上方「刷新」重试。", "Use Refresh above to retry.") }}</p>
    </div>

    <template v-else-if="!loading && settings">
      <section class="agent-runtime-section" data-testid="agent-runtime-section-runtime">
        <header>
          <div class="agent-runtime-section-icon"><Bot :size="16" /></div>
          <div>
            <span class="eyebrow">RUNTIME</span>
            <strong>{{ tr("Runtime 选择", "Runtime selection") }}</strong>
            <p class="helper">
              {{
                tr(
                  "Runtime 负责 Agent 循环与工具请求。选择 Grok CLI 不会复制 provider API Key，也不会启动认证。",
                  "The runtime runs the Agent loop and tool requests. Choosing Grok CLI does not copy a provider API key or start authentication.",
                )
              }}
            </p>
          </div>
        </header>
        <label class="field">
          <span>{{ tr("当前选择", "Current choice") }}</span>
          <select
            v-model="choice"
            data-testid="agent-runtime-choice"
            :disabled="busy"
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
        <p class="helper">{{ selectedHint }}</p>
        <div class="agent-runtime-actions">
          <button
            class="button primary compact"
            type="button"
            data-testid="agent-runtime-save"
            :disabled="busy || choice === settings.runtime.id"
            @click="saveRuntime"
          >
            {{ savingRuntime ? tr("保存中…", "Saving…") : tr("保存 runtime 偏好", "Save runtime preference") }}
          </button>
        </div>
      </section>

      <section class="agent-runtime-section" data-testid="agent-runtime-section-provider">
        <header>
          <div class="agent-runtime-section-icon key"><KeyRound :size="16" /></div>
          <div>
            <span class="eyebrow">xAI API KEY</span>
            <strong>{{ tr("xAI API Key", "xAI API Key") }}</strong>
            <p class="helper">
              {{
                tr(
                  "xAI 对第三方应用提供 API Key 认证（无公开 OAuth 客户端流程）。Key 仅存本机 host，设置接口永不返回明文。",
                  "xAI authenticates third-party apps with API keys (no public OAuth client flow). The key stays on this host; settings never return it.",
                )
              }}
            </p>
          </div>
        </header>
        <div class="form-grid two">
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
        <p class="helper" data-testid="agent-provider-status">
          {{ tr("API Key 状态", "API key status") }} · {{ keyConfiguredLabel }}
        </p>
        <div class="agent-runtime-actions">
          <button
            class="button secondary compact"
            type="button"
            data-testid="agent-provider-save"
            :disabled="
              busy ||
              (!modelDraft.trim() && !apiKeyDraft.trim() && !settings.apiProvider?.apiKeyConfigured)
            "
            @click="saveProvider"
          >
            {{ savingProvider ? tr("保存中…", "Saving…") : tr("保存 xAI provider", "Save xAI provider") }}
          </button>
        </div>
      </section>

      <section class="agent-runtime-section" data-testid="agent-runtime-section-status">
        <header>
          <div>
            <span class="eyebrow">STATUS</span>
            <strong>{{ tr("状态", "Status") }}</strong>
          </div>
        </header>
        <div class="agent-runtime-status-grid">
          <div>
            <span>{{ tr("Host 端点", "Host endpoint") }}</span>
            <div class="agent-runtime-status-value">
              <code>{{ originLabel || "—" }}</code>
              <button
                v-if="originLabel"
                class="button ghost compact"
                type="button"
                data-testid="agent-runtime-copy-origin-status"
                @click="copyOrigin"
              >
                <Copy :size="13" />
                {{ tr("复制", "Copy") }}
              </button>
            </div>
          </div>
          <div>
            <span>{{ tr("Host runtime", "Host runtime") }}</span>
            <strong>
              {{ settings.runtime.label || settings.runtime.id }}
              · {{ settings.runtime.state || "unknown" }}
            </strong>
          </div>
          <div>
            <span>{{ tr("API Key", "API Key") }}</span>
            <strong data-testid="agent-provider-key-badge">{{ keyConfiguredLabel }}</strong>
          </div>
          <div>
            <span>{{ tr("模型 / Base URL", "Model / base URL") }}</span>
            <strong>
              {{ settings.apiProvider?.model || "—" }}
              @ {{ settings.apiProvider?.baseUrl || "—" }}
            </strong>
          </div>
        </div>
      </section>
    </template>

    <div v-else-if="loading" class="empty-state small">
      {{ tr("正在读取 Agent host 设置…", "Reading Agent host settings…") }}
    </div>

    <p v-if="error && !unavailable" class="form-error" role="alert">{{ error }}</p>
    <p v-if="notice" class="helper agent-runtime-notice" role="status">{{ notice }}</p>
  </section>
</template>

<style scoped>
.agent-runtime-settings {
  overflow: hidden;
}

.agent-runtime-alert {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0 18px 18px;
  padding: 12px 14px;
  border: 1px solid color-mix(in srgb, #c58a3a 35%, var(--line));
  border-radius: 9px;
  background: color-mix(in srgb, #fff4eb 88%, var(--app-surface, #fff));
  color: #5d3f24;
  font-size: 12px;
  line-height: 1.55;
  overflow-wrap: anywhere;
}

.agent-runtime-alert strong {
  font-size: 12px;
}

.agent-runtime-alert p {
  margin: 0;
}

.agent-runtime-alert-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.agent-runtime-alert-row > span {
  font-size: 11px;
  color: var(--muted);
}

.agent-runtime-alert-row code {
  font: 11px ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace;
  overflow-wrap: anywhere;
}

.agent-runtime-section {
  display: grid;
  gap: 12px;
  margin: 0 18px 18px;
  padding: 16px;
  border: 1px solid var(--line);
  border-radius: 11px;
  background: color-mix(in srgb, var(--app-surface, #fff) 96%, #f5f7fb);
}

.agent-runtime-section > header {
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

.agent-runtime-section > header > div:last-child {
  display: grid;
  gap: 4px;
  min-width: 0;
}

.agent-runtime-section > header strong {
  font-size: 13px;
}

.agent-runtime-section-icon {
  width: 34px;
  height: 34px;
  flex: 0 0 auto;
  border-radius: 9px;
  display: grid;
  place-items: center;
  background: color-mix(in srgb, var(--blue) 10%, var(--app-surface, #fff));
  color: var(--blue);
}

.agent-runtime-section-icon.key {
  background: color-mix(in srgb, #6f5ed4 10%, var(--app-surface, #fff));
  color: #6f5ed4;
}

.agent-runtime-actions {
  display: flex;
  justify-content: flex-start;
}

.agent-runtime-status-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.agent-runtime-status-grid > div {
  min-width: 0;
  display: grid;
  gap: 5px;
  padding: 11px 12px;
  border: 1px solid var(--line);
  border-radius: 9px;
  background: var(--app-surface, #fff);
}

.agent-runtime-status-grid span {
  font-size: 10px;
  color: var(--muted);
}

.agent-runtime-status-grid strong,
.agent-runtime-status-grid code {
  font-size: 12px;
  overflow-wrap: anywhere;
}

.agent-runtime-status-value {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.agent-runtime-status-value code {
  flex: 1;
  min-width: 0;
  font: 11px ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace;
}

.agent-runtime-notice {
  margin: 0 18px 18px;
}

@media (max-width: 760px) {
  .agent-runtime-status-grid {
    grid-template-columns: 1fr;
  }

  .form-grid.two {
    grid-template-columns: 1fr;
  }
}
</style>
