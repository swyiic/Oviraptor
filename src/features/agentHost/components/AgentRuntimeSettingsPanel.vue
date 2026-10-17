<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { Bot, CheckCircle2, ChevronDown, Copy, KeyRound, LogOut, RefreshCw, Sparkles } from "@lucide/vue";
import { openUrl } from "@tauri-apps/plugin-opener";
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
const oauthBusy = ref(false);
const unavailable = ref(false);
const unavailableReason = ref("");
const error = ref("");
const notice = ref("");
const settings = ref<AgentHostSettings | null>(null);
const choice = ref<AgentRuntimeId>("none");
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
    id: "grok-acp" as const,
    label: tr("Grok CLI · ACP", "Grok CLI · ACP"),
    hint: tr(
      "官方 Grok CLI（ACP）。适合已安装 grok CLI 的环境。",
      "Official Grok CLI (ACP). Use when the grok CLI is installed.",
    ),
  },
  {
    id: "xai-api" as const,
    label: tr("xAI Grok · OAuth / API", "xAI Grok · OAuth / API"),
    hint: tr(
      "使用本机 OAuth 或 API Key，经 api.x.ai 调用模型。",
      "Uses local OAuth or API key against api.x.ai.",
    ),
    disabled: !loggedIn.value,
  },
]);

const selectedHint = computed(
  () => runtimeOptions.value.find((item) => item.id === choice.value)?.hint || "",
);

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
    const value = await client.setRuntime(choice.value);
    applySettings(value);
    notice.value = tr("Runtime 偏好已保存。", "Runtime preference saved.");
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
    <div class="panel-heading">
      <div>
        <span class="eyebrow">AGENT RUNTIME / GROK</span>
        <h3>{{ tr("Agent runtime / Grok", "Agent runtime / Grok") }}</h3>
        <p>
          {{
            tr(
              "一键用 Grok 登录（xAI device-code OAuth）。浏览器授权后凭据写入本机，无需抄端口或粘贴 API Key。",
              "One-click Grok login (xAI device-code OAuth). After browser approval, credentials are stored locally—no port URLs or pasted API keys.",
            )
          }}
        </p>
      </div>
      <div class="hero-actions">
        <button class="button ghost compact" type="button" :disabled="busy" @click="load">
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
        <button class="button ghost compact" type="button" data-testid="agent-runtime-copy-origin" @click="copyOrigin">
          <Copy :size="13" />
          {{ tr("复制", "Copy") }}
        </button>
      </div>
      <p class="helper">{{ tr("可点上方「刷新」重试。", "Use Refresh above to retry.") }}</p>
    </div>

    <template v-else-if="!loading && settings">
      <!-- Primary: Grok OAuth card -->
      <section class="agent-runtime-card login-card" data-testid="agent-runtime-oauth">
        <div class="login-card-top">
          <div class="agent-runtime-section-icon spark"><Sparkles :size="18" /></div>
          <div class="login-card-copy">
            <span class="eyebrow">GROK LOGIN</span>
            <strong>{{ tr("用 Grok 登录", "Sign in with Grok") }}</strong>
            <p class="helper">
              {{
                tr(
                  "复用公开 Grok-CLI OAuth 客户端（device-code）。点按钮打开系统浏览器，确认后本机保存 access/refresh token（0600）。",
                  "Reuses the public Grok-CLI OAuth client (device-code). Opens the system browser; on approval, access/refresh tokens are stored locally (0600).",
                )
              }}
            </p>
          </div>
          <div
            class="login-badge"
            :data-state="oauthLoggedIn ? 'in' : loginStatus === 'pending' ? 'pending' : 'out'"
            data-testid="agent-oauth-badge"
          >
            <CheckCircle2 v-if="oauthLoggedIn" :size="14" />
            {{ authLabel }}
          </div>
        </div>

        <div v-if="loginStatus === 'pending'" class="login-pending" data-testid="agent-oauth-pending">
          <div class="login-code-block">
            <span>{{ tr("登录码", "Login code") }}</span>
            <code data-testid="agent-oauth-user-code">{{ loginUserCode || "—" }}</code>
            <button class="button ghost compact" type="button" :disabled="!loginUserCode" @click="copyUserCode">
              <Copy :size="13" />
              {{ tr("复制", "Copy") }}
            </button>
          </div>
          <p class="helper">
            {{
              tr(
                "若浏览器未自动打开，请手动访问验证页并输入上方短码。正在等待授权…",
                "If the browser did not open, visit the verification page and enter the code above. Waiting for approval…",
              )
            }}
          </p>
          <button
            v-if="loginUri"
            class="button ghost compact"
            type="button"
            data-testid="agent-oauth-reopen"
            @click="openBrowser(loginUri)"
          >
            {{ tr("重新打开浏览器", "Re-open browser") }}
          </button>
        </div>

        <div class="login-actions">
          <button
            v-if="!oauthLoggedIn"
            class="button primary login-cta"
            type="button"
            data-testid="agent-oauth-login"
            :disabled="busy"
            @click="startGrokLogin"
          >
            <Sparkles :size="16" />
            {{
              oauthBusy || loginStatus === "pending"
                ? tr("登录中…", "Signing in…")
                : tr("用 Grok 登录", "Sign in with Grok")
            }}
          </button>
          <button
            v-else
            class="button secondary compact"
            type="button"
            data-testid="agent-oauth-logout"
            :disabled="busy"
            @click="logoutGrok"
          >
            <LogOut :size="14" />
            {{ tr("退出登录", "Sign out") }}
          </button>
        </div>
      </section>

      <!-- Runtime selection -->
      <section class="agent-runtime-section" data-testid="agent-runtime-section-runtime">
        <header>
          <div class="agent-runtime-section-icon"><Bot :size="16" /></div>
          <div>
            <span class="eyebrow">RUNTIME</span>
            <strong>{{ tr("Runtime 选择", "Runtime selection") }}</strong>
            <p class="helper">
              {{
                tr(
                  "登录成功后会自动切到 xAI Grok runtime。也可手动选择 Grok CLI（ACP）。",
                  "Successful login switches to the xAI Grok runtime. You can also pick Grok CLI (ACP).",
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

      <!-- Advanced: API key collapsed -->
      <section class="agent-runtime-section advanced" data-testid="agent-runtime-section-provider">
        <button
          class="advanced-toggle"
          type="button"
          data-testid="agent-advanced-toggle"
          :aria-expanded="advancedOpen"
          @click="advancedOpen = !advancedOpen"
        >
          <div class="agent-runtime-section-icon key"><KeyRound :size="16" /></div>
          <div class="advanced-toggle-copy">
            <span class="eyebrow">ADVANCED</span>
            <strong>{{ tr("高级 · API Key", "Advanced · API key") }}</strong>
            <p class="helper">
              {{
                tr(
                  "仅在无法使用 OAuth 时需要。主路径是上方「用 Grok 登录」。",
                  "Only needed when OAuth is unavailable. Prefer Sign in with Grok above.",
                )
              }}
            </p>
          </div>
          <ChevronDown :size="16" :class="{ open: advancedOpen }" />
        </button>
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
            {{ tr("API Key 状态", "API key status") }} ·
            {{
              settings.apiProvider?.apiKeyConfigured
                ? tr("已配置", "Configured")
                : tr("未配置", "Not configured")
            }}
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
              {{ savingProvider ? tr("保存中…", "Saving…") : tr("保存 API Key", "Save API key") }}
            </button>
          </div>
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
            <span>{{ tr("登录", "Sign-in") }}</span>
            <strong data-testid="agent-provider-key-badge">{{ authLabel }}</strong>
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

.agent-runtime-card,
.agent-runtime-section {
  display: grid;
  gap: 12px;
  margin: 0 18px 18px;
  padding: 16px;
  border: 1px solid var(--line);
  border-radius: 11px;
  background: color-mix(in srgb, var(--app-surface, #fff) 96%, #f5f7fb);
}

.login-card {
  background:
    linear-gradient(135deg, color-mix(in srgb, var(--blue) 8%, transparent), transparent 55%),
    color-mix(in srgb, var(--app-surface, #fff) 96%, #f5f7fb);
}

.login-card-top {
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

.login-card-copy {
  display: grid;
  gap: 4px;
  min-width: 0;
  flex: 1;
}

.login-card-copy strong {
  font-size: 14px;
}

.login-badge {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px;
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
  background: color-mix(in srgb, #fff4eb 88%, var(--app-surface, #fff));
}

.login-pending {
  display: grid;
  gap: 10px;
  padding: 12px;
  border-radius: 9px;
  border: 1px dashed color-mix(in srgb, var(--blue) 35%, var(--line));
  background: color-mix(in srgb, var(--blue) 4%, var(--app-surface, #fff));
}

.login-code-block {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
}

.login-code-block > span {
  font-size: 11px;
  color: var(--muted);
}

.login-code-block code {
  font: 600 18px ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace;
  letter-spacing: 0.08em;
  padding: 6px 10px;
  border-radius: 8px;
  background: var(--app-surface, #fff);
  border: 1px solid var(--line);
}

.login-actions {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
}

.login-cta {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-height: 40px;
  padding: 0 18px;
  font-weight: 600;
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

.agent-runtime-section-icon.spark {
  background: color-mix(in srgb, #111 92%, var(--blue));
  color: #fff;
}

.agent-runtime-actions {
  display: flex;
  justify-content: flex-start;
}

.advanced-toggle {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  width: 100%;
  margin: 0;
  padding: 0;
  border: 0;
  background: transparent;
  text-align: left;
  cursor: pointer;
  color: inherit;
}

.advanced-toggle-copy {
  display: grid;
  gap: 4px;
  min-width: 0;
  flex: 1;
}

.advanced-toggle svg:last-child {
  margin-top: 8px;
  color: var(--muted);
  transition: transform 0.15s ease;
}

.advanced-toggle svg:last-child.open {
  transform: rotate(180deg);
}

.advanced-body {
  display: grid;
  gap: 12px;
  padding-top: 4px;
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

  .login-card-top {
    flex-wrap: wrap;
  }
}
</style>
