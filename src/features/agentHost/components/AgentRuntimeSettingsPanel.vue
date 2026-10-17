<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { CheckCircle2, Copy, LogOut, RefreshCw, Sparkles } from "@lucide/vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useI18n } from "../../../i18n";
import { openAgentHostSettingsClient } from "../api";
import type { AgentAnswerMode, AgentHostSettings, AgentRuntimeId, HostSettingsClient } from "../api";

const emit = defineEmits<{
  notify: [type: "success" | "error" | "info", text: string];
}>();

const { tr } = useI18n();

const loading = ref(true);
const savingRuntime = ref(false);
const savingProvider = ref(false);
const oauthBusy = ref(false);
const unavailable = ref(false);
const unavailableReason = ref("");
const error = ref("");
const notice = ref("");
const settings = ref<AgentHostSettings | null>(null);
const choice = ref<AgentRuntimeId>("none");
const answerMode = ref<AgentAnswerMode>("quality");
const modelDraft = ref("");
const apiKeyDraft = ref("");
const originLabel = ref("");
const advancedOpen = ref(false);
const loginUserCode = ref("");
const loginUri = ref("");
const loginStatus = ref<"idle" | "pending" | "authorized" | "denied" | "expired" | "error">("idle");

let client: HostSettingsClient | null = null;
let pollTimer: ReturnType<typeof setInterval> | null = null;

const loggedIn = computed(
  () => Boolean(settings.value?.apiProvider?.oauthConfigured || settings.value?.apiProvider?.apiKeyConfigured),
);
const oauthLoggedIn = computed(() => Boolean(settings.value?.apiProvider?.oauthConfigured));
const authLabel = computed(() => {
  if (settings.value?.apiProvider?.oauthConfigured) {
    return tr("已用 Grok 登录", "Signed in with Grok");
  }
  if (settings.value?.apiProvider?.apiKeyConfigured) {
    return tr("已配置 API Key", "API key configured");
  }
  return tr("未登录", "Not signed in");
});

const runtimeOptions = computed(() => [
  {
    id: "none" as const,
    label: tr("无 · 仅本地消息", "None · local messages only"),
    hint: tr("不启动 Agent runtime；对话仍可用。", "No Agent runtime; chat remains available."),
  },
  {
    id: "xai-api" as const,
    label: tr("xAI Grok · OAuth（问答主路径）", "xAI Grok · OAuth (primary Q&A)"),
    hint: tr(
      "默认问答主路径：本机 OAuth（或 API Key）经 api.x.ai；含质量护栏 / Responses / 验收。",
      "Primary Q&A path: local OAuth (or API key) via api.x.ai with quality harness / Responses / verifier.",
    ),
    disabled: !loggedIn.value,
  },
  {
    id: "grok-acp" as const,
    label: tr("Grok CLI · ACP（高级）", "Grok CLI · ACP (advanced)"),
    hint: tr(
      "高级/CLI：官方 Grok CLI（ACP）。不走 host 侧质量护栏；适合已安装 grok CLI 的环境。",
      "Advanced/CLI: official Grok CLI (ACP). Host quality harness does not apply; use when grok CLI is installed.",
    ),
  },
]);

const busy = computed(
  () => loading.value || savingRuntime.value || savingProvider.value || oauthBusy.value,
);

function notify(type: "success" | "error" | "info", text: string) {
  emit("notify", type, text);
}

function stopPoll() {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
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

async function copyUserCode() {
  const value = loginUserCode.value?.trim();
  if (!value) return;
  try {
    await navigator.clipboard.writeText(value);
    notice.value = tr("已复制登录码", "Login code copied");
    notify("info", notice.value);
  } catch {
    notice.value = value;
  }
}

function applySettings(value: AgentHostSettings) {
  settings.value = value;
  choice.value = value.runtime.id;
  if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent("oviraptor:agent-host-settings"));
  answerMode.value = value.answerMode === "speed" ? "speed" : "quality";
  modelDraft.value = value.apiProvider?.model || modelDraft.value;
  if (value.apiProvider?.oauthConfigured) {
    loginStatus.value = "authorized";
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
    applySettings(value);
    apiKeyDraft.value = "";
    if (value.apiProvider?.oauthConfigured) loginStatus.value = "authorized";
    else if (loginStatus.value === "authorized") loginStatus.value = "idle";
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
    const value = await client.setRuntime(choice.value, { answerMode: answerMode.value });
    applySettings(value);
    notice.value = tr("Runtime / 问答档位已保存。", "Runtime / answer mode saved.");
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
    applySettings(value);
    apiKeyDraft.value = "";
    notice.value = tr(
      "高级 provider 已保存。凭据仅存本机 host（0600）。",
      "Advanced provider saved. Credentials stay on this host (0600).",
    );
    notify("success", notice.value);
  } catch (err) {
    error.value = String((err as Error)?.message || err);
    notify("error", error.value);
  } finally {
    savingProvider.value = false;
  }
}

async function openBrowser(url: string) {
  if (!url) return;
  try {
    await openUrl(url);
  } catch {
    try {
      window.open(url, "_blank", "noopener,noreferrer");
    } catch {
      notice.value = tr(`请在浏览器打开：${url}`, `Open in browser: ${url}`);
    }
  }
}

function oauthErrorText(code: string): string {
  if (code === "host_provider_network_unreachable") {
    return tr("连不上 auth.x.ai，请检查代理（系统代理或 HTTPS_PROXY）。", "Cannot reach auth.x.ai. Check your proxy (system proxy or HTTPS_PROXY).");
  }
  return code;
}

async function startGrokLogin() {
  if (!client || oauthBusy.value) return;
  oauthBusy.value = true;
  error.value = "";
  notice.value = "";
  stopPoll();
  try {
    const result = await client.startOAuth({ referrer: "oviraptor" });
    applySettings(result);
    loginUserCode.value = result.userCode || result.oauth?.userCode || "";
    loginUri.value =
      result.verificationUriComplete ||
      result.oauth?.verificationUriComplete ||
      result.verificationUri ||
      result.oauth?.verificationUri ||
      "";
    loginStatus.value = "pending";
    if (loginUri.value) await openBrowser(loginUri.value);
    notice.value = tr(
      "已打开浏览器。在页面确认后，本机会自动完成登录。",
      "Browser opened. After you confirm, this host finishes login automatically.",
    );
    notify("info", notice.value);
    pollTimer = setInterval(() => {
      void pollLogin();
    }, 2500);
  } catch (err) {
    loginStatus.value = "error";
    error.value = oauthErrorText(String((err as Error)?.message || err));
    notify("error", error.value);
  } finally {
    oauthBusy.value = false;
  }
}

async function pollLogin() {
  if (!client) return;
  try {
    const result = await client.pollOAuth();
    applySettings(result);
    const status = (result.status || result.oauth?.status || "pending") as typeof loginStatus.value;
    if (status === "authorized" || result.apiProvider?.oauthConfigured) {
      loginStatus.value = "authorized";
      stopPoll();
      notice.value = tr("Grok 登录成功。已切换到 xAI runtime。", "Grok login succeeded. Switched to xAI runtime.");
      notify("success", notice.value);
      return;
    }
    if (status === "denied" || status === "expired" || status === "error") {
      loginStatus.value = status;
      stopPoll();
      error.value = oauthErrorText(result.error || status);
      notify("error", error.value);
    } else {
      loginStatus.value = "pending";
    }
  } catch (err) {
    // Transient poll errors: keep waiting until expiry.
    console.warn("oauth poll", err);
  }
}

async function logoutGrok() {
  if (!client || oauthBusy.value) return;
  oauthBusy.value = true;
  error.value = "";
  stopPoll();
  try {
    const value = await client.logoutOAuth();
    applySettings(value);
    loginStatus.value = "idle";
    loginUserCode.value = "";
    loginUri.value = "";
    notice.value = tr("已退出 Grok 登录。", "Signed out of Grok.");
    notify("success", notice.value);
  } catch (err) {
    error.value = String((err as Error)?.message || err);
    notify("error", error.value);
  } finally {
    oauthBusy.value = false;
  }
}

onMounted(() => {
  void load();
});

onUnmounted(() => {
  stopPoll();
});
</script>

<template>
  <section class="panel agent-runtime-settings" data-testid="agent-runtime-settings">
    <div class="config-section-head">
      <h3>{{ tr("运行时与模型", "Runtime & model") }}</h3>
      <div class="actions">
        <button class="button ghost compact" type="button" :disabled="busy" @click="load">
          <RefreshCw :size="13" :class="{ spinning: loading }" />
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
      <span>{{ unavailableReason }}</span>
      <div v-if="originLabel" class="agent-runtime-alert-row" data-testid="agent-runtime-failed-origin">
        <code>{{ originLabel }}</code>
        <button class="button ghost compact" type="button" data-testid="agent-runtime-copy-origin" @click="copyOrigin">
          <Copy :size="13" />{{ tr("复制", "Copy") }}
        </button>
      </div>
    </div>

    <template v-else-if="!loading && settings">
      <div class="runtime-bar" data-testid="agent-runtime-oauth">
        <div class="runtime-bar-main">
          <strong>{{ tr("Grok", "Grok") }}</strong>
          <span
            class="login-badge"
            :data-state="oauthLoggedIn ? 'in' : loginStatus === 'pending' ? 'pending' : 'out'"
            data-testid="agent-oauth-badge"
          >
            <CheckCircle2 v-if="oauthLoggedIn" :size="12" />
            {{ authLabel }}
          </span>
        </div>
        <div class="runtime-bar-actions">
          <button
            v-if="!oauthLoggedIn"
            class="button primary compact"
            type="button"
            data-testid="agent-oauth-login"
            :disabled="busy"
            @click="startGrokLogin"
          >
            <Sparkles :size="14" />
            {{
              oauthBusy || loginStatus === "pending"
                ? tr("登录中…", "Signing in…")
                : tr("用 Grok 登录", "Sign in with Grok")
            }}
          </button>
          <button
            v-else
            class="button ghost compact"
            type="button"
            data-testid="agent-oauth-logout"
            :disabled="busy"
            @click="logoutGrok"
          >
            <LogOut :size="13" />
            {{ tr("退出", "Sign out") }}
          </button>
        </div>
      </div>

      <div v-if="loginStatus === 'pending'" class="login-pending" data-testid="agent-oauth-pending">
        <div class="login-code-block">
          <span>{{ tr("登录码", "Code") }}</span>
          <code data-testid="agent-oauth-user-code">{{ loginUserCode || "—" }}</code>
          <button class="button ghost compact" type="button" :disabled="!loginUserCode" @click="copyUserCode">
            <Copy :size="13" />{{ tr("复制", "Copy") }}
          </button>
          <button
            v-if="loginUri"
            class="button ghost compact"
            type="button"
            data-testid="agent-oauth-reopen"
            @click="openBrowser(loginUri)"
          >{{ tr("打开浏览器", "Open browser") }}</button>
        </div>
      </div>

      <div class="config-form-row" data-testid="agent-runtime-section-runtime">
        <span>{{ tr("Runtime", "Runtime") }}</span>
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
          >{{ option.label }}</option>
        </select>
      </div>
      <div class="config-form-row">
        <span>{{ tr("问答档位", "Answer mode") }}</span>
        <select
          v-model="answerMode"
          data-testid="agent-answer-mode"
          :disabled="busy"
          :aria-label="tr('问答档位', 'Answer mode')"
        >
          <option value="quality">{{ tr("质量（默认）", "Quality (default)") }}</option>
          <option value="speed">{{ tr("速度", "Speed") }}</option>
        </select>
      </div>
      <div class="config-form-actions">
        <button
          class="button primary compact"
          type="button"
          data-testid="agent-runtime-save"
          :disabled="busy || (choice === settings.runtime.id && answerMode === (settings.answerMode || 'quality'))"
          @click="saveRuntime"
        >{{ savingRuntime ? tr("保存中…", "Saving…") : tr("保存", "Save") }}</button>
      </div>

      <details class="advanced-fold" data-testid="agent-runtime-section-provider" :open="advancedOpen || undefined">
        <summary
          data-testid="agent-advanced-toggle"
          @click.prevent="advancedOpen = !advancedOpen"
        >{{ tr("高级 · API Key", "Advanced · API key") }}</summary>
        <div v-if="advancedOpen" class="advanced-body" data-testid="agent-advanced-body">
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
                :placeholder="settings.apiProvider?.apiKeyConfigured ? tr('（保持不变）', '(unchanged)') : 'xai-…'"
                :aria-label="tr('xAI API Key', 'xAI API key')"
              />
            </label>
          </div>
          <div class="config-form-actions">
            <button
              class="button secondary compact"
              type="button"
              data-testid="agent-provider-save"
              :disabled="busy || (!modelDraft.trim() && !apiKeyDraft.trim() && !settings.apiProvider?.apiKeyConfigured)"
              @click="saveProvider"
            >{{ savingProvider ? tr("保存中…", "Saving…") : tr("保存 API Key", "Save API key") }}</button>
          </div>
        </div>
      </details>

      <p class="config-inline-status" data-testid="agent-runtime-section-status">
        {{ settings.runtime.label || settings.runtime.id }}
        · {{ settings.runtime.state || "—" }}
        · {{ authLabel }}
        · {{ settings.apiProvider?.model || "—" }}
        <template v-if="originLabel"> · <code data-testid="agent-runtime-copy-origin-status">{{ originLabel }}</code></template>
      </p>
    </template>

    <div v-else-if="loading" class="empty-state small">{{ tr("读取中…", "Loading…") }}</div>
    <p v-if="error && !unavailable" class="form-error" role="alert">{{ error }}</p>
    <p v-if="notice" class="helper agent-runtime-notice" role="status">{{ notice }}</p>
  </section>
</template>

<style scoped>
.agent-runtime-settings { padding: 16px; }
.config-section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}
.config-section-head h3 { margin: 0; font-size: 13px; font-weight: 600; }
.config-section-head .actions { display: flex; gap: 6px; }
.config-form-row {
  display: grid;
  grid-template-columns: 100px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  margin: 0 0 10px;
}
.config-form-row > span { font-size: 12px; color: var(--muted); }
.config-form-actions { display: flex; justify-content: flex-end; gap: 8px; margin: 4px 0 12px; }
.config-inline-status {
  margin: 0;
  padding-top: 10px;
  border-top: 1px solid var(--line);
  font-size: 11px;
  color: var(--muted);
  line-height: 1.5;
  overflow-wrap: anywhere;
}
.config-inline-status code { font-size: 10px; }

.runtime-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 14px;
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: 9px;
  background: color-mix(in srgb, var(--blue) 5%, var(--app-surface, #fff));
}
.runtime-bar-main { display: flex; align-items: center; gap: 10px; min-width: 0; }
.runtime-bar-main strong { font-size: 13px; }
.runtime-bar-actions { display: flex; gap: 8px; flex-shrink: 0; }

.login-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border-radius: 999px;
  border: 1px solid var(--line);
  font-size: 11px;
  color: var(--muted);
  background: var(--app-surface, #fff);
}
.login-badge[data-state="in"] {
  color: #1f6b3a;
  border-color: color-mix(in srgb, #1f6b3a 30%, var(--line));
  background: color-mix(in srgb, #e8f7ee 90%, var(--app-surface, #fff));
}
.login-badge[data-state="pending"] {
  color: #8a5a12;
  border-color: color-mix(in srgb, #c58a3a 35%, var(--line));
}
.login-pending {
  margin: 0 0 12px;
  padding: 10px 12px;
  border: 1px dashed color-mix(in srgb, var(--blue) 35%, var(--line));
  border-radius: 8px;
}
.login-code-block { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
.login-code-block > span { font-size: 11px; color: var(--muted); }
.login-code-block code {
  font: 600 15px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  letter-spacing: 0.06em;
  padding: 4px 8px;
  border-radius: 6px;
  border: 1px solid var(--line);
  background: var(--app-surface, #fff);
}

.advanced-fold { margin: 4px 0 12px; border-top: 1px solid var(--line); padding-top: 8px; }
.advanced-fold > summary {
  list-style: none;
  cursor: pointer;
  font-size: 12px;
  font-weight: 550;
  color: var(--muted);
  user-select: none;
}
.advanced-fold > summary::-webkit-details-marker { display: none; }
.advanced-fold > summary:hover { color: var(--app-ink, #172033); }
.advanced-body { display: grid; gap: 10px; margin-top: 10px; }

.agent-runtime-alert {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 12px;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, #c58a3a 35%, var(--line));
  border-radius: 8px;
  background: color-mix(in srgb, #fff4eb 88%, var(--app-surface, #fff));
  color: #5d3f24;
  font-size: 12px;
}
.agent-runtime-alert-row { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
.agent-runtime-alert-row code { font-size: 11px; overflow-wrap: anywhere; }
.agent-runtime-notice { margin: 8px 0 0; font-size: 11px; }

@media (max-width: 720px) {
  .config-form-row { grid-template-columns: 1fr; }
  .runtime-bar { flex-wrap: wrap; }
  .form-grid.two { grid-template-columns: 1fr; }
}
</style>
