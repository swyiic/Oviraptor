<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { Bot, CheckCircle2, ChevronDown, Copy, KeyRound, LogOut, RefreshCw, Sparkles } from "@lucide/vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useI18n } from "../../../i18n";
import { openAgentHostSettingsClient } from "../api";
import type { AgentAnswerMode, AgentHostSettings, AgentRuntimeId, HostSettingsClient } from "../api";
import { DEEPSEEK_BASE_URL, XAI_BASE_URL, providerDisplayName } from "../providerPresets";
import type { AgentNetworkMode, AgentNetworkRoute } from "../types";

const emit = defineEmits<{
  notify: [type: "success" | "error" | "info", text: string];
}>();

const { tr } = useI18n();

const loading = ref(true);
const savingRuntime = ref(false);
const savingProvider = ref(false);
const savingNetwork = ref(false);
const networkMode = ref<AgentNetworkMode>("system");
const networkProxyDraft = ref("");
const networkNoProxy = ref("");
const oauthBusy = ref(false);
const unavailable = ref(false);
const unavailableReason = ref("");
const error = ref("");
const notice = ref("");
const settings = ref<AgentHostSettings | null>(null);
const choice = ref<AgentRuntimeId>("none");
const answerMode = ref<AgentAnswerMode>("quality");
const modelDraft = ref("");
const providerKindDraft = ref<"xai" | "deepseek" | "custom">("xai");
const baseUrlDraft = ref(XAI_BASE_URL);
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
const oauthLoggedIn = computed(() => Boolean(settings.value?.apiProvider?.oauthSaved || settings.value?.apiProvider?.oauthConfigured));
const oauthLabel = computed(() => settings.value?.apiProvider?.oauthConfigured ? tr("当前使用 Grok OAuth", "Grok OAuth active") : oauthLoggedIn.value ? tr("Grok 登录已保留", "Grok sign-in saved") : tr("尚未登录 Grok", "Grok not signed in"));
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
    label: tr("模型 API · 使用下方选择的连接", "Model API · use the selected connection"),
    hint: tr(
      "连接由服务商、地址、模型与认证组成，可使用 Grok OAuth 或独立 API Key。",
      "A connection consists of provider, endpoint, model and authentication: Grok OAuth or a separate API key.",
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
  () => loading.value || savingRuntime.value || savingProvider.value || savingNetwork.value || oauthBusy.value,
);
const networkDirty = computed(() => networkProxyDraft.value.length > 0 || networkMode.value !== (settings.value?.network?.mode || "system") || networkNoProxy.value !== (settings.value?.network?.noProxy || ""));
const networkRoutes = computed(() => [
  { label: tr("模型请求", "Model requests"), route: settings.value?.network?.routes.provider },
  { label: tr("Grok OAuth", "Grok OAuth"), route: settings.value?.network?.routes.oauth },
]);

function networkRouteLabel(route: AgentNetworkRoute): string {
  if (route.source === "manual-proxy") return tr("Host 手动代理", "Host manual proxy");
  if (route.source === "inherited-environment") return `${tr("继承环境代理", "Inherited environment proxy")} · ${route.environmentVariable || ""}`;
  if (route.reason === "loopback") return tr("本机地址直连", "Loopback: direct");
  if (route.reason === "no-proxy") return tr("绕过列表：直连", "Bypass rule: direct");
  if (route.reason === "unsupported-environment-proxy") return tr("环境代理无效或不支持：直连", "Unsupported environment proxy: direct");
  return tr("直接连接", "Direct connection");
}

async function saveNetwork(clearProxy = false) {
  if (!client || savingNetwork.value) return;
  savingNetwork.value = true; error.value = ""; notice.value = "";
  try {
    const mode = clearProxy && networkMode.value === "manual" ? "system" : networkMode.value;
    const value = await client.setNetwork({ mode, noProxy: networkNoProxy.value, ...(clearProxy ? { clearProxy: true } : networkProxyDraft.value.trim() ? { proxyUrl: networkProxyDraft.value.trim() } : {}) });
    applySettings(value);
    networkProxyDraft.value = ""; networkMode.value = value.network?.mode || "system"; networkNoProxy.value = value.network?.noProxy || "";
    notice.value = clearProxy ? tr("已清除保存的手动代理。", "Saved manual proxy removed.") : tr("Host 网络设置已保存，后续模型与 OAuth 请求使用此设置。", "Host network settings saved for subsequent model and OAuth requests.");
    notify("success", notice.value);
  } catch (reason) {
    const code = String((reason as Error)?.message || reason);
    const messages: Record<string, string> = {
      host_network_proxy_invalid: tr("请输入有效的 HTTP(S) 代理地址；地址不能带路径、查询参数或片段。", "Enter a valid HTTP(S) proxy without a path, query or fragment."),
      host_network_proxy_required: tr("手动模式需要填写或保留一个代理地址。", "Manual mode needs a new or saved proxy endpoint."),
      host_network_bypass_invalid: tr("绕过列表仅接受主机名、IP、可选端口与通配域名，使用逗号分隔。", "Use comma-separated hosts, IPs, optional ports or wildcard domains."),
      host_runtime_busy: tr("请等当前问答或 OAuth 授权结束后再修改网络。", "Wait for the current turn or OAuth authorization before changing the network."),
    };
    error.value = messages[code] || code; notify("error", error.value);
  } finally { savingNetwork.value = false; }
}

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
  const updateNetworkDraft = !settings.value || !networkDirty.value;
  settings.value = value;
  if (updateNetworkDraft) { networkMode.value = value.network?.mode || "system"; networkNoProxy.value = value.network?.noProxy || ""; }
  choice.value = value.runtime.id;
  if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent("oviraptor:agent-host-settings"));
  answerMode.value = value.answerMode === "speed" ? "speed" : "quality";
  modelDraft.value = value.apiProvider?.model || modelDraft.value;
  baseUrlDraft.value = value.apiProvider?.baseUrl || XAI_BASE_URL;
  providerKindDraft.value = /api\.deepseek\.com/i.test(baseUrlDraft.value) ? "deepseek" : value.apiProvider?.provider === "openai-compatible" ? "custom" : "xai";
  if (value.apiProvider?.oauthConfigured) {
    loginStatus.value = "authorized";
  }
}

function selectProvider() {
  apiKeyDraft.value = "";
  if (providerKindDraft.value === "xai") { baseUrlDraft.value = XAI_BASE_URL; modelDraft.value = "grok-4.7"; }
  else if (providerKindDraft.value === "deepseek") { baseUrlDraft.value = DEEPSEEK_BASE_URL; modelDraft.value = "deepseek-v4-flash"; }
  else { baseUrlDraft.value = ""; modelDraft.value = ""; }
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
      provider: providerKindDraft.value === "xai" ? "xai" : "openai-compatible",
      baseUrl: baseUrlDraft.value.trim() || undefined,
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

async function useSavedOAuth() {
  if (!client || savingProvider.value) return;
  savingProvider.value = true; error.value = "";
  try { applySettings(await client.selectOAuth()); }
  catch (reason) { error.value = String((reason as Error)?.message || reason); }
  finally { savingProvider.value = false; }
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
    return tr("连不上 auth.x.ai，请检查下方 Host 网络的请求路径。", "Cannot reach auth.x.ai. Check the Host network route below.");
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
        <h3>{{ tr("聊天模型连接", "Chat model connections") }}</h3>
        <p class="helper">{{tr('聊天连接与扫描模型、资产 API、业务代理分别保存。','Chat connections save separately from scan models, asset APIs and business proxies.')}}</p>
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
            <strong>{{ tr("用 Grok 登录", "Sign in with Grok") }}</strong>
          </div>
          <div
            class="login-badge"
            :data-state="oauthLoggedIn ? 'in' : loginStatus === 'pending' ? 'pending' : 'out'"
            data-testid="agent-oauth-badge"
          >
            <CheckCircle2 v-if="oauthLoggedIn" :size="14" />
            {{ oauthLabel }}
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
          <button v-if="oauthLoggedIn&&!settings.apiProvider?.oauthConfigured" type="button" class="button primary compact" :disabled="busy" @click="useSavedOAuth">{{tr('使用已保存的 Grok 登录','Use saved Grok sign-in')}}</button>
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
            <strong>{{ tr("Runtime 选择", "Runtime selection") }}</strong>
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
        <label class="field">
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
        </label>
        <div class="agent-runtime-actions">
          <button
            class="button primary compact"
            type="button"
            data-testid="agent-runtime-save"
            :disabled="busy || (choice === settings.runtime.id && answerMode === (settings.answerMode || 'quality'))"
            @click="saveRuntime"
          >
            {{ savingRuntime ? tr("保存中…", "Saving…") : tr("保存", "Save") }}
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
            <strong>{{ tr("服务商、模型与 API Key", "Provider, model & API key") }}</strong>
          </div>
          <ChevronDown :size="16" :class="{ open: advancedOpen }" />
        </button>
        <div v-if="advancedOpen" class="advanced-body" data-testid="agent-advanced-body">
          <div class="form-grid two">
            <label class="field"><span>{{tr('服务商','Provider')}}</span><select v-model="providerKindDraft" :disabled="busy" data-testid="agent-provider-kind" @change="selectProvider"><option value="xai">xAI Grok</option><option value="deepseek">DeepSeek</option><option value="custom">{{tr('自定义 OpenAI 兼容','Custom OpenAI compatible')}}</option></select></label>
            <label class="field"><span>{{tr('模型服务地址','Model endpoint')}}</span><input v-model="baseUrlDraft" :readonly="providerKindDraft!=='custom'" :disabled="busy" placeholder="https://example.com/v1" data-testid="agent-provider-base-url" /></label>
            <label class="field">
              <span>{{ tr("模型", "Model") }}</span>
              <input
                v-model="modelDraft"
                data-testid="agent-xai-model"
                placeholder="grok-4.7 / deepseek-v4-flash"
                autocomplete="off"
                :aria-label="tr('模型 ID', 'Model ID')"
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
                :aria-label="tr('当前服务商 API Key', 'Selected provider API key')"
              />
            </label>
          </div>
          <p class="helper" data-testid="agent-provider-status">
            {{ providerDisplayName(settings.apiProvider?.provider,settings.apiProvider?.baseUrl) }} · {{ tr("当前连接 API Key 状态", "Active connection API key status") }} ·
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

      <section class="agent-runtime-section host-network" data-testid="agent-runtime-section-network">
        <header><strong>{{ tr("聊天 Host 网络", "Chat Host network") }}</strong></header>
        <p class="network-note">{{ tr("仅用于聊天模型 API 与 Grok OAuth；扫描、资产 API 和 Grok CLI/ACP 使用各自的网络设置。", "Applies to chat model APIs and Grok OAuth. Scans, asset APIs and Grok CLI/ACP use their own network settings.") }}</p>
        <template v-if="settings.network">
          <p v-if="!settings.network.valid" class="form-error">{{ tr("保存的 Host 网络配置无效，请重新保存。", "Saved Host network settings are invalid. Save a replacement.") }}</p>
          <label class="field"><span>{{ tr("连接方式", "Connection mode") }}</span><select v-model="networkMode" :disabled="busy" data-testid="agent-network-mode"><option value="system">{{ tr("继承 Host 启动环境", "Inherit Host startup environment") }}</option><option value="direct">{{ tr("Host 直接连接", "Host direct connection") }}</option><option value="manual">{{ tr("Host 手动 HTTP(S) 代理", "Host manual HTTP(S) proxy") }}</option></select></label>
          <p v-if="networkMode === 'system'" class="network-note">{{ tr("读取 Host 启动时继承的代理环境；桌面启动器可能注入系统代理，当前不能区分其原始来源。系统设置变更后需重启 Host。", "Uses proxy environment inherited when Host started. The desktop launcher may inject system proxies; their original source cannot be distinguished. Restart Host after changing system settings.") }}</p>
          <template v-if="networkMode === 'manual'">
            <label class="field"><span>{{ tr("代理地址（可含认证）", "Proxy URL (authentication optional)") }}</span><input v-model="networkProxyDraft" type="password" autocomplete="off" :disabled="busy" placeholder="http://127.0.0.1:7890" data-testid="agent-network-proxy" /></label>
            <p class="network-note">{{ tr("仅支持 HTTP(S) 代理；留空保留已保存地址，认证信息保存后不回显。", "HTTP(S) proxies only. Leave blank to keep the saved URL. Saved credentials are never returned.") }}</p>
            <label class="field"><span>{{ tr("直接连接的主机", "Hosts that bypass the proxy") }}</span><textarea v-model="networkNoProxy" rows="2" maxlength="4096" :disabled="busy" placeholder=".internal.example,example.com:443" data-testid="agent-network-no-proxy" /></label>
          </template>
          <p v-if="settings.network.manualProxyConfigured" class="network-note" data-testid="agent-network-saved-proxy">{{ tr("保存的手动代理", "Saved manual proxy") }}：<code>{{ settings.network.manualProxyEndpoint }}</code> · {{ settings.network.manualProxyCredentialsConfigured ? tr("含认证", "Authentication configured") : tr("无认证", "No authentication") }}</p>
          <div class="agent-runtime-actions"><button type="button" class="button primary compact" :disabled="busy || (!networkDirty && settings.network.valid)" data-testid="agent-network-save" @click="saveNetwork()">{{ savingNetwork ? tr("保存中…", "Saving…") : tr("保存 Host 网络", "Save Host network") }}</button><button v-if="settings.network.manualProxyConfigured" type="button" class="button ghost compact" :disabled="busy" data-testid="agent-network-clear" @click="saveNetwork(true)">{{ tr("清除手动代理", "Remove manual proxy") }}</button></div>
          <p v-if="settings.network.transport === 'injected'" class="network-note">{{ tr("当前使用注入的测试传输；此传输未使用 Host 网络策略。", "An injected test transport is active; Host network policy does not route that transport.") }}</p>
          <div class="network-routes" data-testid="agent-network-routes">
            <strong>{{ tr("下次请求路径", "Next request routes") }}</strong>
            <template v-for="item in networkRoutes" :key="item.label"><p v-if="item.route" class="network-note"><span>{{ item.label }} · {{ item.route.target }}</span><strong>{{ networkRouteLabel(item.route) }}</strong><code v-if="item.route.proxyEndpoint">{{ item.route.proxyEndpoint }}</code></p></template>
            <p v-if="settings.network.lastRequest" class="network-note" data-testid="agent-network-last-request"><span>{{ tr("最近实际请求", "Last actual request") }} · {{ settings.network.lastRequest.target }}</span><strong>{{ networkRouteLabel(settings.network.lastRequest) }}</strong><span>{{ settings.network.lastRequest.outcome === 'response' ? `HTTP ${settings.network.lastRequest.httpStatus}` : settings.network.lastRequest.outcome === 'error' ? tr("连接失败", "Connection failed") : settings.network.lastRequest.outcome === 'cancelled' ? tr("已取消", "Cancelled") : tr("进行中", "In progress") }}</span></p>
          </div>
        </template>
        <p v-else class="network-note">{{ tr("当前 Host 尚未提供独立网络配置，请更新 Host。", "This Host does not expose independent network settings. Update Host to use them.") }}</p>
      </section>

      <section class="agent-runtime-section" data-testid="agent-runtime-section-status">
        <header>
          <div>
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
  padding: 16px;
}
.agent-runtime-settings > .panel-heading {
  margin: 0 0 12px;
  padding: 0;
}
.agent-runtime-settings > .panel-heading p { display: none; }
.agent-runtime-settings .helper { display: none; }
.agent-runtime-settings .login-pending .helper { display: block; font-size: 11px; }
.network-note { margin: 0; color: var(--muted); font-size: 12px; line-height: 1.55; overflow-wrap: anywhere; }
.network-routes { display: grid; gap: 8px; }
.network-routes > .network-note { display: grid; gap: 2px; }
.network-routes strong { color: var(--text); font-size: 12px; }
.host-network code { overflow-wrap: anywhere; }


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
  gap: 10px;
  margin: 0 0 14px;
  padding: 0 0 14px;
  border: 0;
  border-bottom: 1px solid var(--line);
  border-radius: 0;
  background: transparent;
}

.login-card { background: transparent; }

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
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.agent-runtime-status-grid > div {
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 8px 12px;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
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
