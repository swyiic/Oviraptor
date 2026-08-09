<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import {
  Check,
  Database,
  GitBranch,
  KeyRound,
  Plus,
  RefreshCw,
  Settings2,
  Trash2,
} from "@lucide/vue";
import ModalShell from "./ModalShell.vue";
import { api } from "../api";
import type { ConfigProfile, SecurityRulePack } from "../types";
import { useI18n } from "../i18n";

const props = defineProps<{ profile?: ConfigProfile }>();
const emit = defineEmits<{ close: []; saved: [id: number] }>();
const { tr } = useI18n();

interface StrixLlmProfile {
  id: string;
  name: string;
  llm: string;
  apiBase: string;
  apiKey: string;
  deployment: "cloud" | "local";
}

let strixProfileSequence = 0;
function newStrixLlmProfile(name = ""): StrixLlmProfile {
  strixProfileSequence += 1;
  return {
    id:
      globalThis.crypto?.randomUUID?.() ||
      `strix-model-${Date.now()}-${strixProfileSequence}`,
    name: name || tr("新模型", "New model"),
    llm: "",
    apiBase: "",
    apiKey: "",
    deployment: "cloud",
  };
}

const defaults = {
  pythonExecutable: "python3",
  redisCliExecutable: "",
  strixExecutable: "",
  strixRunsDirectory: "~/strix_runs",
  strixLlm: "",
  strixApiBase: "",
  strixApiKey: "",
  strixLlmProfiles: [] as StrixLlmProfile[],
  strixActiveLlmProfileId: "",
  strixLocalFullPower: false,
  strixFrontendPacketMode: "balanced",
  strixFrontendPacketBudgetKb: 12,
  strixPromptAuditMode: "off",
  windowsRuntimeDirectory: "C:\\oviraptor\\runtime",
  scriptsDirectory: "",
  configPath: "",
  fofaEmail: "",
  fofaKey: "",
  hackerOneUsername: "",
  hackerOneToken: "",
  proxyUrl: "",
  noProxy: "127.0.0.1,localhost",
  strixBatchSize: 15,
  strixQuickScore: 30,
  strixStandardScore: 55,
  strixDeepScore: 80,
  strixQuickTimeout: 120,
  strixStandardTimeout: 300,
  strixDeepTimeout: 600,
  strixQuickTokenLimit: 50000,
  strixStandardTokenLimit: 120000,
  strixDeepTokenLimit: 250000,
  strixQuickRequestLimit: 4,
  strixStandardRequestLimit: 8,
  strixDeepRequestLimit: 12,
  strixNoToolTurnLimit: 4,
  strixProxyEnabled: false,
  authorizedProxyPool: [] as string[],
  collectionMode: "all",
  fofaProfile: "professional",
  pageSize: 500,
  maxPages: 0,
  maxDerivedDomains: 200,
  interval: 6,
  collectionTimeout: 45,
  fullHistory: false,
  enableCidr24: false,
  includeWeakFingerprints: false,
  runRefine: true,
  runProbe: true,
  includeOther: true,
  includeWeak: false,
  priorityRate: 20,
  otherRate: 10,
  workers: 64,
  probeTimeout: 6,
  probeRetries: 0,
  contentThreshold: 12,
  gamblingKeywords: ["在线赌博", "博彩平台", "真人视讯", "体育投注"],
  pornKeywords: ["色情网站", "成人网站", "成人视频", "情色直播"],
  negativeKeywords: ["打击赌博", "扫黄打非", "反诈", "公安", "法院"],
  replaceDefaultContentRules: false,
  semgrepRuleRepository: "",
  semgrepRuleReference: "",
  codeqlRuleRepository: "",
  codeqlRuleReference: "",
  owaspBenchmarkRepository: "",
  owaspBenchmarkReference: "",
};
const storedSettings = props.profile?.settings ?? {};
function normalizedStrixProfiles(): StrixLlmProfile[] {
  if (Array.isArray(storedSettings.strixLlmProfiles)) {
    return storedSettings.strixLlmProfiles
      .filter((value: any) => value && typeof value === "object")
      .map((value: any, index: number) => ({
        id: String(value.id || `strix-model-${index + 1}`),
        name: String(value.name || `${tr("模型", "Model")} ${index + 1}`),
        llm: String(value.llm || ""),
        apiBase: String(value.apiBase || ""),
        apiKey: String(value.apiKey || ""),
        deployment: value.deployment === "local" ? "local" : "cloud",
      }));
  }
  if (
    storedSettings.strixLlm ||
    storedSettings.strixApiBase ||
    storedSettings.strixApiKey
  ) {
    return [
      {
        id: "legacy-default",
        name: tr("默认模型", "Default model"),
        llm: String(storedSettings.strixLlm || ""),
        apiBase: String(storedSettings.strixApiBase || ""),
        apiKey: String(storedSettings.strixApiKey || ""),
        deployment: "cloud",
      },
    ];
  }
  return [];
}
const initialStrixProfiles = normalizedStrixProfiles();
const form = reactive({
  name: props.profile?.name ?? tr("新配置", "New profile"),
  description: props.profile?.description ?? "",
  isDefault: props.profile?.isDefault ?? false,
  settings: {
    ...defaults,
    ...storedSettings,
    strixLlmProfiles: initialStrixProfiles,
    strixActiveLlmProfileId:
      String(storedSettings.strixActiveLlmProfileId || "") ||
      initialStrixProfiles[0]?.id ||
      "",
  },
});
const tab = ref<
  | "accounts"
  | "runtime"
  | "strix"
  | "collection"
  | "probe"
  | "rules"
  | "securityRules"
>("accounts");
const busy = ref(false);
const error = ref("");
const rulePacks = ref<SecurityRulePack[]>([]);
const ruleBusy = ref("");
const ruleMessage = ref("");
const modelTestBusy = ref(false);
const modelTestStatus = ref<"idle" | "passed" | "failed">("idle");
const modelTestMessage = ref("");
const fofaTestBusy = ref(false);
const fofaTestStatus = ref<"idle" | "passed" | "failed">("idle");
const fofaTestMessage = ref("");
const modelTestKeys = reactive<Record<string, string>>({});
const initialModelTestKeys = initialStrixProfiles.reduce<Record<string, string>>(
  (keys, profile) => {
    keys[profile.id] = profileTestSignatureSource(profile);
    return keys;
  },
  {},
);
const ruleDefinitions = [
  {
    key: "semgrep-rules",
    name: "Semgrep Rules",
    engine: "semgrep",
    repositoryKey: "semgrepRuleRepository",
    referenceKey: "semgrepRuleReference",
    builtinRepository: "https://github.com/semgrep/semgrep-rules.git",
    builtinReference: "develop",
    description: "多语言模式匹配、污点分析和安全规则。",
  },
  {
    key: "codeql-queries",
    name: "CodeQL Queries",
    engine: "codeql",
    repositoryKey: "codeqlRuleRepository",
    referenceKey: "codeqlRuleReference",
    builtinRepository: "https://github.com/github/codeql.git",
    builtinReference: "main",
    description: "跨过程数据流、控制流和语义查询。",
  },
  {
    key: "owasp-benchmark",
    name: "OWASP Benchmark",
    engine: "benchmark",
    repositoryKey: "owaspBenchmarkRepository",
    referenceKey: "owaspBenchmarkReference",
    builtinRepository: "https://github.com/OWASP/Benchmark.git",
    builtinReference: "master",
    description: "用于规则覆盖率与误报回归测试，不直接作为漏洞规则执行。",
  },
] as const;
const keywordText = computed({
  get: () => (form.settings.gamblingKeywords ?? []).join("\n"),
  set: (value) => {
    form.settings.gamblingKeywords = value
      .split("\n")
      .map((v) => v.trim())
      .filter(Boolean);
  },
});
const pornText = computed({
  get: () => (form.settings.pornKeywords ?? []).join("\n"),
  set: (value) => {
    form.settings.pornKeywords = value
      .split("\n")
      .map((v) => v.trim())
      .filter(Boolean);
  },
});
const negativeText = computed({
  get: () => (form.settings.negativeKeywords ?? []).join("\n"),
  set: (value) => {
    form.settings.negativeKeywords = value
      .split("\n")
      .map((v) => v.trim())
      .filter(Boolean);
  },
});
const authorizedProxyText = computed({
  get: () => (form.settings.authorizedProxyPool ?? []).join("\n"),
  set: (value) => {
    form.settings.authorizedProxyPool = value
      .split("\n")
      .map((v) => v.trim())
      .filter(Boolean);
  },
});
const activeStrixProfile = computed(() =>
  form.settings.strixLlmProfiles.find(
    (profile) => profile.id === form.settings.strixActiveLlmProfileId,
  ),
);
const localFullPowerActive = computed(
  () =>
    Boolean(form.settings.strixLocalFullPower) &&
    activeStrixProfile.value?.deployment === "local",
);

function profileTestSignatureSource(profile: StrixLlmProfile) {
  // The display name is local metadata and must not invalidate a connectivity test.
  return `${profile.id}\u0000${profile.llm}\u0000${profile.apiBase}\u0000${profile.apiKey}\u0000${profile.deployment}`;
}
function profileHasAcceptedTest(profile: StrixLlmProfile) {
  const signature = profileTestSignatureSource(profile);
  return (
    signature === modelTestKeys[profile.id] ||
    signature === initialModelTestKeys[profile.id]
  );
}
async function testActiveStrixProfile() {
  const profile = activeStrixProfile.value;
  if (!profile) return;
  modelTestBusy.value = true;
  modelTestStatus.value = "idle";
  modelTestMessage.value = "";
  try {
    const result = await api.testStrixLlm({
      llm: profile.llm.trim(),
      deployment: profile.deployment,
      apiBase: profile.apiBase.trim(),
      apiKey: profile.apiKey,
    });
    // Keep the same raw signature format used by profileHasAcceptedTest.
    modelTestKeys[profile.id] = profileTestSignatureSource(profile);
    modelTestStatus.value = "passed";
    modelTestMessage.value = result.message;
  } catch (reason) {
    modelTestStatus.value = "failed";
    modelTestMessage.value = String(reason);
  } finally {
    modelTestBusy.value = false;
  }
}

async function testFofaApi() {
  fofaTestBusy.value = true;
  fofaTestStatus.value = "idle";
  fofaTestMessage.value = "";
  try {
    const result = await api.testFofaApi({
      key: String(form.settings.fofaKey || "").trim(),
      proxyUrl: String(form.settings.proxyUrl || "").trim(),
    });
    fofaTestStatus.value = "passed";
    const detail = [result.account, result.plan].filter(Boolean).join(" · ");
    fofaTestMessage.value = `${result.message}${detail ? ` · ${detail}` : ""}`;
  } catch (reason) {
    fofaTestStatus.value = "failed";
    fofaTestMessage.value = String(reason);
  } finally {
    fofaTestBusy.value = false;
  }
}

function activateStrixProfile(id: string) {
  form.settings.strixActiveLlmProfileId = id;
  syncLegacyStrixFields();
  const profile = activeStrixProfile.value;
  const accepted = profile ? profileHasAcceptedTest(profile) : false;
  modelTestStatus.value = accepted ? "passed" : "idle";
  modelTestMessage.value = accepted
    ? tr("该模型已通过连通性测试。", "This model passed the connectivity test.")
    : "";
}
function addStrixProfile() {
  const profile = newStrixLlmProfile();
  form.settings.strixLlmProfiles.push(profile);
  activateStrixProfile(profile.id);
}
function removeStrixProfile(id: string) {
  const index = form.settings.strixLlmProfiles.findIndex(
    (profile) => profile.id === id,
  );
  if (index < 0) return;
  form.settings.strixLlmProfiles.splice(index, 1);
  if (form.settings.strixActiveLlmProfileId === id) {
    form.settings.strixActiveLlmProfileId =
      form.settings.strixLlmProfiles[Math.min(index, form.settings.strixLlmProfiles.length - 1)]
        ?.id || "";
  }
  syncLegacyStrixFields();
  delete modelTestKeys[id];
  delete initialModelTestKeys[id];
  modelTestStatus.value =
    activeStrixProfile.value && profileHasAcceptedTest(activeStrixProfile.value)
      ? "passed"
      : "idle";
  modelTestMessage.value = "";
}

watch(
  () => activeStrixProfile.value?.deployment,
  (deployment) => {
    if (deployment !== "local" && form.settings.strixLocalFullPower) {
      form.settings.strixLocalFullPower = false;
    }
  },
);
watch(
  () => (activeStrixProfile.value ? profileTestSignatureSource(activeStrixProfile.value) : ""),
  () => {
    const profile = activeStrixProfile.value;
    if (!profile) {
      modelTestStatus.value = "idle";
      modelTestMessage.value = "";
      return;
    }
    if (profileHasAcceptedTest(profile)) {
      modelTestStatus.value = "passed";
      modelTestMessage.value = tr(
        "该模型已通过连通性测试。",
        "This model passed the connectivity test.",
      );
      return;
    }
    if (modelTestStatus.value === "passed") {
      modelTestStatus.value = "idle";
      modelTestMessage.value = "";
    }
  },
);
function syncLegacyStrixFields() {
  const profile = activeStrixProfile.value;
  form.settings.strixLlm = profile?.llm.trim() || "";
  form.settings.strixApiBase = profile?.apiBase.trim() || "";
  form.settings.strixApiKey = profile?.apiKey.trim() || "";
}

function configuredValue(key: string) {
  return String((form.settings as Record<string, unknown>)[key] ?? "").trim();
}
function rulePack(key: string) {
  return rulePacks.value.find((pack) => pack.key === key);
}
function ruleStatus(pack?: SecurityRulePack) {
  if (!pack || pack.status === "not_installed")
    return tr("未安装", "Not installed");
  if (pack.status === "syncing") return tr("同步中", "Syncing");
  if (pack.status === "ready") return tr("已就绪", "Ready");
  return tr("错误", "Error");
}
async function loadRulePacks() {
  try {
    rulePacks.value = await api.listSecurityRulePacks();
  } catch (reason) {
    error.value = String(reason);
  }
}
async function persistRulePack(definition: (typeof ruleDefinitions)[number]) {
  return api.saveSecurityRulePack({
    key: definition.key,
    name: definition.name,
    engine: definition.engine,
    repository: configuredValue(definition.repositoryKey),
    reference: configuredValue(definition.referenceKey),
    enabled: true,
  });
}
async function syncRulePack(definition: (typeof ruleDefinitions)[number]) {
  ruleBusy.value = definition.key;
  error.value = "";
  ruleMessage.value = "";
  try {
    const id = await persistRulePack(definition);
    await api.syncSecurityRulePack(id);
    ruleMessage.value = `${definition.name} ${tr("已进入后台同步", "is syncing in the background")}`;
    for (let attempt = 0; attempt < 600; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 700));
      await loadRulePacks();
      const pack = rulePack(definition.key);
      if (pack?.status === "ready") {
        ruleMessage.value = `${definition.name} ${tr("同步完成", "synchronized")}：${pack.previousVersion || "首次安装"} -> ${pack.version || "—"}`;
        return;
      }
      if (pack?.status === "error") throw new Error(pack.error || tr("规则库同步失败", "Rule sync failed"));
    }
    throw new Error(tr("规则库仍在后台同步，请稍后回到配置中心查看", "Rule sync is still running; check Configuration later"));
  } catch (reason) {
    error.value = String(reason);
  } finally {
    ruleBusy.value = "";
  }
}

async function save() {
  busy.value = true;
  error.value = "";
  try {
    syncLegacyStrixFields();
    if (activeStrixProfile.value) {
      if (!profileHasAcceptedTest(activeStrixProfile.value)) {
        throw new Error(
          tr(
            "请先完成当前 Strix 模型连通性测试，测试通过后才能保存配置。",
            "Run and pass the current Strix model connectivity test before saving.",
          ),
        );
      }
    }
    await Promise.all(ruleDefinitions.map(persistRulePack));
    const id = await api.saveProfile({
      id: props.profile?.id && props.profile.id > 0 ? props.profile.id : undefined,
      ...form,
    });
    emit("saved", id);
  } catch (reason) {
    error.value = String(reason);
  } finally {
    busy.value = false;
  }
}
onMounted(loadRulePacks);
</script>

<template>
  <ModalShell
    :title="tr('配置方案', 'Configuration profile')"
    wide
    @close="$emit('close')"
  >
    <template #eyebrow
      ><span class="eyebrow"><Settings2 :size="14" /> PROFILE</span></template
    >
    <div class="profile-heading">
      <label class="field grow"
        ><span>{{ tr("配置名称", "Profile name") }}</span
        ><input v-model="form.name"
      /></label>
      <label class="check-field"
        ><input v-model="form.isDefault" type="checkbox" />
        {{ tr("设为默认", "Set as default") }}</label
      >
    </div>
    <label class="field"
      ><span>{{ tr("说明", "Description") }}</span
      ><input v-model="form.description"
    /></label>
    <p v-if="error" class="form-error profile-error-top">{{ error }}</p>
    <div class="tab-strip">
      <button
        v-for="item in [
          ['accounts', tr('账号与 API', 'Accounts & API')],
          ['runtime', tr('运行环境', 'Runtime')],
          ['strix', tr('Strix 策略', 'Strix policy')],
          ['collection', tr('采集', 'Collection')],
          ['probe', tr('探测', 'Probe')],
          ['rules', tr('内容规则', 'Content rules')],
          ['securityRules', tr('代码规则库', 'Code rules')],
        ]"
        :key="item[0]"
        :class="{ active: tab === item[0] }"
        @click="tab = item[0] as any"
      >
        {{ item[1] }}
      </button>
    </div>
    <div v-if="tab === 'accounts'" class="account-settings">
      <div class="account-settings-intro">
        <KeyRound :size="17" />
        <div>
          <strong>{{ tr("账号与 API", "Accounts and APIs") }}</strong>
          <span>{{
            tr(
              "模型和第三方平台凭据集中保存在当前配置方案中。切换 Strix 模型后，应用会直接为新任务注入环境变量，不需要修改或 source ~/.zshrc。",
              "Model and provider credentials live in this profile. Switching the Strix model injects environment variables into new tasks directly; no ~/.zshrc edit or source command is required.",
            )
          }}</span>
        </div>
      </div>

      <section class="account-provider-section strix-model-section">
        <header>
          <div>
            <strong>{{ tr("Strix 模型配置", "Strix model profiles") }}</strong>
            <span>{{
              tr(
                "保存多个 OpenAI 兼容模型，启用标记决定下一次扫描使用哪一个。",
                "Save multiple OpenAI-compatible models; the active marker controls the next scan.",
              )
            }}</span>
          </div>
          <button
            class="button ghost compact"
            type="button"
            @click="addStrixProfile"
          >
            <Plus :size="14" />{{ tr("添加模型", "Add model") }}
          </button>
        </header>
        <p v-if="error" class="form-error model-error-inline">{{ error }}</p>

        <div v-if="form.settings.strixLlmProfiles.length" class="model-switch-list">
          <button
            v-for="profile in form.settings.strixLlmProfiles"
            :key="profile.id"
            type="button"
            :class="{ active: profile.id === form.settings.strixActiveLlmProfileId }"
            :aria-pressed="profile.id === form.settings.strixActiveLlmProfileId"
            @click="activateStrixProfile(profile.id)"
          >
            <span>
              <strong>{{ profile.name || tr("未命名模型", "Unnamed model") }}</strong>
              <small>{{ profile.deployment === "local" ? tr("本地自建", "Self-hosted") : tr("云端", "Cloud") }} · {{ profile.llm || tr("尚未填写模型 ID", "Model ID not set") }}</small>
            </span>
            <em v-if="profile.id === form.settings.strixActiveLlmProfileId">
              <Check :size="12" />{{ tr("当前启用", "Active") }}
            </em>
            <em v-else>{{ tr("切换", "Switch") }}</em>
          </button>
        </div>
        <div v-else class="model-profile-empty">
          {{ tr("还没有模型配置。添加后即可在这里切换。", "No model profiles yet. Add one to enable switching.") }}
        </div>

        <div v-if="activeStrixProfile" class="model-profile-editor">
          <header>
            <div>
              <strong>{{ tr("编辑当前模型", "Edit active model") }}</strong>
              <span>{{ activeStrixProfile.name }}</span>
            </div>
            <div class="model-editor-actions">
              <button
                class="button danger compact model-delete-button"
                type="button"
                :title="tr('删除此模型', 'Delete this model')"
                @click="removeStrixProfile(activeStrixProfile.id)"
              >
                <Trash2 :size="14" />{{ tr("删除模型", "Delete model") }}
              </button>
              <button
                class="button secondary compact model-test-button"
                type="button"
                :disabled="modelTestBusy"
                @click="testActiveStrixProfile"
              >
                <RefreshCw :size="14" :class="{ spinning: modelTestBusy }" />
                {{ modelTestBusy ? tr("测试中", "Testing") : tr("测试模型", "Test model") }}
              </button>
            </div>
          </header>
          <div class="form-grid two">
            <label class="field"
              ><span>{{ tr("显示名称", "Display name") }}</span
              ><input v-model="activeStrixProfile.name" placeholder="DeepSeek V4"
            /></label>
            <label class="field"
              ><span>STRIX_LLM</span
              ><input v-model="activeStrixProfile.llm" placeholder="deepseek/deepseek-v4-pro"
            /></label>
            <label class="field span-two"
              ><span>{{ tr("部署类型", "Deployment") }}</span
              ><select
                v-model="activeStrixProfile.deployment"
              >
                <option value="cloud">{{ tr("云端 API", "Cloud API") }}</option>
                <option value="local">{{ tr("本地自建 OpenAI 兼容服务", "Self-hosted OpenAI-compatible") }}</option>
              </select>
              <small>{{
                activeStrixProfile.deployment === "local"
                  ? tr("本地服务允许 Key 留空；请填写可从本机访问的 Base URL。", "API key may be blank for a local service; enter a Base URL reachable from this machine.")
                  : tr("云端模型继续使用 Strix 自适应预算与熔断策略。", "Cloud models continue to use adaptive Strix budgets and fuses.")
              }}</small>
            </label>
            <label class="field span-two"
              ><span>OPENAI_BASE_URL</span
              ><input
                v-model="activeStrixProfile.apiBase"
                placeholder="https://api.example.com/v1"
            /></label>
            <label class="field span-two"
              ><span>OPENAI_API_KEY</span
              ><input
                v-model="activeStrixProfile.apiKey"
                type="password"
                autocomplete="new-password"
                :placeholder="activeStrixProfile.deployment === 'local' ? tr('本地服务可留空', 'Optional for local service') : tr('留空时读取 Strix CLI 配置', 'Blank uses the Strix CLI config')"
            /></label>
            <small v-if="activeStrixProfile.deployment === 'local'" class="helper span-two">{{ tr("上下文窗口、最大输出和推理参数完全由 MLX 本地服务管理；Oviraptor 不再覆盖这些参数。", "Context size, maximum output, and inference parameters are managed entirely by the local MLX service; Oviraptor no longer overrides them.") }}</small>
          </div>
          <p v-if="modelTestMessage" class="model-test-message" :class="modelTestStatus">
            <Check v-if="modelTestStatus === 'passed'" :size="13" />{{ modelTestMessage }}
          </p>
        </div>
      </section>

      <section class="account-provider-section">
        <header>
          <div>
            <strong>FOFA</strong>
            <span>{{ tr("资产检索账号", "Asset search account") }}</span>
          </div>
          <button class="button secondary compact" type="button" :disabled="fofaTestBusy" @click="testFofaApi">
            <RefreshCw :size="14" :class="{ spinning: fofaTestBusy }" />
            {{ fofaTestBusy ? tr("探测中", "Testing") : tr("探测 API", "Test API") }}
          </button>
        </header>
        <div class="form-grid two">
          <label class="field"
            ><span>FOFA account / email</span
            ><input v-model="form.settings.fofaEmail" autocomplete="username"
          /></label>
          <label class="field"
            ><span>FOFA key</span
            ><input
              v-model="form.settings.fofaKey"
              type="password"
              autocomplete="new-password"
              :placeholder="tr('仅保存在本机应用数据库', 'Stored only in the local app database')"
          /></label>
          <label class="field span-two"
            ><span>{{ tr("旧版 FOFA config 路径（Key 留空时使用）", "Legacy FOFA config path (used when key is blank)") }}</span
            ><input v-model="form.settings.configPath"
          /></label>
        </div>
        <p v-if="fofaTestMessage" class="model-test-message" :class="fofaTestStatus">
          <Check v-if="fofaTestStatus === 'passed'" :size="13" />{{ fofaTestMessage }}
        </p>
      </section>

      <section class="account-provider-section">
        <header>
          <div>
            <strong>HackerOne</strong>
            <span>{{ tr("漏洞平台 API 账号", "Vulnerability platform API account") }}</span>
          </div>
        </header>
        <div class="form-grid two">
          <label class="field"
            ><span>HackerOne API identifier</span
            ><input v-model="form.settings.hackerOneUsername" autocomplete="username"
          /></label>
          <label class="field"
            ><span>HackerOne API token</span
            ><input
              v-model="form.settings.hackerOneToken"
              type="password"
              autocomplete="new-password"
              :placeholder="tr('Token 值，仅保存在本机', 'Token value, local only')"
          /></label>
        </div>
      </section>
      <p class="helper account-secret-note">
        {{
          tr(
            "凭据仅保存在本机 SQLite；任务日志与任务快照不会写入明文 Key。",
            "Credentials remain in local SQLite; plaintext keys are excluded from task logs and snapshots.",
          )
        }}
      </p>
    </div>
    <div v-else-if="tab === 'runtime'" class="form-grid two">
      <label class="field"
        ><span>Python executable</span
        ><input v-model="form.settings.pythonExecutable" placeholder="python3"
      /></label>
      <label class="field"
        ><span>redis-cli executable</span
        ><input
          v-model="form.settings.redisCliExecutable"
          :placeholder="
            tr(
              '留空自动检测；Windows 可填写完整 exe 路径',
              'Blank for auto-detect; Windows can use a full exe path',
            )
          "
      /></label>
      <label class="field"
        ><span>Strix executable</span
        ><input
          v-model="form.settings.strixExecutable"
          :placeholder="
            tr(
              '留空自动检测 ~/.strix/bin/strix',
              'Blank to auto-detect ~/.strix/bin/strix',
            )
          "
      /></label>
      <label class="field"
        ><span>Strix runs directory</span
        ><input
          v-model="form.settings.strixRunsDirectory"
          placeholder="~/strix_runs"
      /></label>
      <label class="field"
        ><span>{{
          tr(
            "Windows runtime directory（建议）",
            "Windows runtime directory (recommended)",
          )
        }}</span
        ><input
          v-model="form.settings.windowsRuntimeDirectory"
          placeholder="C:\oviraptor\runtime"
      /></label>
      <label class="field"
        ><span>{{
          tr(
            "Scripts directory（留空使用内置脚本）",
            "Scripts directory (blank uses bundled workers)",
          )
        }}</span
        ><input
          v-model="form.settings.scriptsDirectory"
          :placeholder="
            tr('留空使用内置 1–7 脚本', 'Leave blank for bundled workers 1–7')
          "
      /></label>
      <label class="field"
        ><span>{{ tr("代理地址（Clash）", "Proxy URL (Clash)") }}</span
        ><input
          v-model="form.settings.proxyUrl"
          placeholder="http://127.0.0.1:7890"
      /></label>
      <label class="field"
        ><span>NO_PROXY</span
        ><input
          v-model="form.settings.noProxy"
          placeholder="127.0.0.1,localhost"
      /></label>
      <div class="proxy-pool-setting span-two">
        <label class="check-field"
          ><input v-model="form.settings.strixProxyEnabled" type="checkbox" />
          {{
            tr(
              "启用自有 / 已授权代理池",
              "Enable owned / authorized proxy pool",
            )
          }}</label
        >
        <label class="field"
          ><span>{{
            tr(
              "授权代理（每行一个，可用 国家标签|URL）",
              "Authorized proxies (one per line; COUNTRY|URL)",
            )
          }}</span
          ><textarea
            v-model="authorizedProxyText"
            rows="4"
            placeholder="CN|http://127.0.0.1:7890&#10;GLOBAL|socks5://127.0.0.1:1080"
          ></textarea>
        </label>
        <p class="helper">
          {{
            tr(
              "仅接受你拥有或明确获准使用的代理。Web 资产任务会按子批次轮换；模型 API 如需直连，请把其域名加入 NO_PROXY。",
              "Only use proxies you own or are explicitly allowed to use. Web batches rotate through the list; add model API hosts to NO_PROXY when they must connect directly.",
            )
          }}
        </p>
      </div>
      <p class="helper span-two">
        {{
          tr(
            "采集脚本必须使用安装了 pandas 的 Python；如果系统 python3 不同，请填写虚拟环境 bin/python 的完整路径。",
            "The collector needs a Python with pandas; if system python3 differs, enter the full path to your virtualenv bin/python.",
          )
        }}
      </p>
      <p class="helper span-two">
        {{
          tr(
            "Windows 建议把 Python/Redis 工具放到 C:\\oviraptor\\runtime，或分别填写 python.exe 与 redis-cli.exe 的完整路径；无需修改系统 PATH。",
            "On Windows, place Python/Redis tools under C:\\oviraptor\\runtime or enter full paths to python.exe and redis-cli.exe; system PATH changes are optional.",
          )
        }}
      </p>
    </div>
    <div v-else-if="tab === 'strix'" class="strix-policy-settings">
      <div class="skill-format-guide">
        <strong>{{ tr("Strix 自适应分流", "Adaptive Strix routing") }}</strong
        ><span>{{
          tr(
            "低价值仅保留前端结果；有价值目标按单 URL 自动使用 quick / standard / deep。",
            "Low-value targets keep frontend results only; valuable targets run quick / standard / deep one URL at a time.",
          )
        }}</span>
      </div>
      <div class="strix-governance-grid">
        <article class="strix-governance-card">
          <div>
            <strong>{{ tr("本地模型火力全开", "Local model full power") }}</strong>
            <span>{{ tr("仅当当前启用模型标记为“本地自建”时生效。", "Only applies when the active model is marked self-hosted.") }}</span>
          </div>
          <label class="switch-control">
            <input
              v-model="form.settings.strixLocalFullPower"
              type="checkbox"
              :disabled="!activeStrixProfile || activeStrixProfile.deployment !== 'local'"
            />
            <span></span>
          </label>
          <p>{{
            tr(
              "开启后使用本地模型算力，但不再强制 deep 或放宽到数小时；目标仍按证据自适应分流，并在模型、Token、工具和日志持续无进展时自动结束当前 URL。",
              "Uses local model capacity without forcing deep mode or multi-hour limits. Targets keep evidence-based adaptive routing and stop the current URL when model, token, tool, and log progress stalls.",
            )
          }}</p>
        </article>
        <article class="strix-governance-card">
          <div>
            <strong>{{ tr("本地 LLM Hook 与提示词审计", "Local LLM hook and prompt audit") }}</strong>
            <span>{{ tr("Token 始终统计；这里控制新任务保存多少请求内容。", "Tokens are always counted; this controls how much request content new tasks retain.") }}</span>
          </div>
          <label class="field">
            <select
              v-model="form.settings.strixPromptAuditMode"
            >
              <option value="off">{{ tr("仅统计 Token", "Token counts only") }}</option>
              <option value="metadata">{{ tr("请求元数据与哈希", "Request metadata and hashes") }}</option>
              <option value="full">{{ tr("保存本机完整模型请求", "Store full model requests locally") }}</option>
            </select>
          </label>
          <p>{{
            tr(
              "本地自建模型通过回环 Hook 采集。全文档位会在本机原样记录 Strix 最终组装的 system / developer / user / tool schema、请求内容与响应摘要，仅对超长字段做截断；云模型仍只保留 Oviraptor 生成的 instruction。",
              "Self-hosted models are captured through a loopback hook. Full mode stores Strix's assembled system, developer, user, tool schema, request content, and response summary verbatim on this device, truncating only oversized fields. Cloud models retain only the Oviraptor-generated instruction.",
            )
          }}</p>
        </article>
      </div>
      <section class="strix-packet-policy">
        <div>
          <strong>{{ tr("发送给 Strix 的前端数据预算", "Frontend packet budget for Strix") }}</strong>
          <span>{{ tr("统一限制 frontend-evidence.json 与代码片段总量；URL、状态、API、参数、路由、敏感线索和运行时信号按优先级保留。", "Caps frontend-evidence.json and code slices together; URLs, status, APIs, parameters, routes, sensitive clues, and runtime signals are retained by priority.") }}</span>
        </div>
        <div class="form-grid two">
          <label class="field"
            ><span>{{ tr("压缩策略", "Compression strategy") }}</span
            ><select v-model="form.settings.strixFrontendPacketMode">
              <option value="balanced">{{ tr("均衡（使用自定义预算）", "Balanced (use custom budget)") }}</option>
              <option value="compact">{{ tr("紧凑（固定 6 KB）", "Compact (fixed 6 KB)") }}</option>
              <option value="custom">{{ tr("自定义预算", "Custom budget") }}</option>
            </select></label
          >
          <label class="field"
            ><span>{{ tr("前端数据总预算（KB）", "Frontend packet budget (KB)") }}</span
            ><input v-model.number="form.settings.strixFrontendPacketBudgetKb" type="number" min="4" max="64" step="1"
          /></label>
        </div>
        <p class="helper">{{ tr("8K 上下文建议 4–6 KB；32K/40K 上下文建议 12–24 KB。预算只裁剪发送给 Strix 的前端文件，不删除本地完整探测结果。", "For an 8K context use 4–6 KB; for 32K/40K use 12–24 KB. This only trims frontend files sent to Strix; complete recon results stay local.") }}</p>
      </section>
      <fieldset class="strix-policy-controls" :disabled="localFullPowerActive">
      <div class="form-grid two">
        <label class="field"
          ><span>{{ tr("前端预解析每批 URL 数", "Frontend recon URLs per batch") }}</span
          ><input v-model.number="form.settings.strixBatchSize" type="number" min="1" max="50"
        /></label>
        <label class="field"
          ><span>{{ tr("无进展熔断轮次", "No-progress request limit") }}</span
          ><input v-model.number="form.settings.strixNoToolTurnLimit" type="number" min="1" max="100"
        /></label>
      </div>
      <div class="strix-policy-table">
        <div class="strix-policy-head">
          <span>{{ tr("模式", "Mode") }}</span>
          <span>{{ tr("起始评分", "Score") }}</span>
          <span>{{ tr("请求软预算", "Requests") }}</span>
          <span>{{ tr("超时（秒）", "Timeout (s)") }}</span>
          <span>{{ tr("新增处理 Token", "Uncached + output") }}</span>
        </div>
        <div>
          <strong>Quick</strong>
          <input v-model.number="form.settings.strixQuickScore" type="number" min="1" max="90" />
          <input v-model.number="form.settings.strixQuickRequestLimit" type="number" min="1" max="100" />
          <input v-model.number="form.settings.strixQuickTimeout" type="number" min="30" />
          <input v-model.number="form.settings.strixQuickTokenLimit" type="number" min="0" step="10000" />
        </div>
        <div>
          <strong>Standard</strong>
          <input v-model.number="form.settings.strixStandardScore" type="number" min="2" max="95" />
          <input v-model.number="form.settings.strixStandardRequestLimit" type="number" min="1" max="200" />
          <input v-model.number="form.settings.strixStandardTimeout" type="number" min="60" />
          <input v-model.number="form.settings.strixStandardTokenLimit" type="number" min="0" step="10000" />
        </div>
        <div>
          <strong>Deep</strong>
          <input v-model.number="form.settings.strixDeepScore" type="number" min="3" max="100" />
          <input v-model.number="form.settings.strixDeepRequestLimit" type="number" min="1" max="300" />
          <input v-model.number="form.settings.strixDeepTimeout" type="number" min="120" />
          <input v-model.number="form.settings.strixDeepTokenLimit" type="number" min="0" step="10000" />
        </div>
      </div>
      <p class="helper">
        {{
          localFullPowerActive
            ? tr(
                "本地火力全开只切换执行模型，不覆盖这里保存的 quick / standard / deep 策略。启动阶段连续 180 秒无进展会失败；进入扫描后按最后一次有效进展动态熔断，总时长仅作为最终保险丝。",
                "Local full power removes token, request-count, active-duration, repeated-tool, and no-progress limits. Startup, model-interface, and context-window failures remain protected.",
              )
            : tr(
                "Token 上限按新增输入 + 输出计算；填 0 只关闭这一层预算，累计上下文绝对上限、请求次数、重复工具和无进展保护仍然生效。",
                "Token limits count uncached input plus output; 0 disables only this budget layer. Absolute cumulative-context, request-count, repeated-tool, and no-progress safeguards still apply.",
              )
        }}
      </p>
      </fieldset>
    </div>
    <div v-else-if="tab === 'collection'" class="form-grid three">
      <label class="field"
        ><span>Collection mode</span
        ><select v-model="form.settings.collectionMode">
          <option>all</option>
          <option>discover</option>
          <option>expand</option>
        </select></label
      >
      <label class="field"
        ><span>FOFA profile</span
        ><select v-model="form.settings.fofaProfile">
          <option>professional</option>
          <option>business</option>
          <option>basic</option>
          <option>auto</option>
        </select></label
      >
      <label class="field"
        ><span>Page size</span
        ><input v-model.number="form.settings.pageSize" type="number"
      /></label>
      <label class="field"
        ><span>Interval (s)</span
        ><input
          v-model.number="form.settings.interval"
          type="number"
          step="0.5"
      /></label>
      <label class="field"
        ><span>Timeout (s)</span
        ><input v-model.number="form.settings.collectionTimeout" type="number"
      /></label>
      <label class="field"
        ><span>Max pages (0=all)</span
        ><input v-model.number="form.settings.maxPages" type="number"
      /></label>
      <label class="field"
        ><span>{{
          tr("派生域名回查上限/公司", "Derived domain limit/company")
        }}</span
        ><input
          v-model.number="form.settings.maxDerivedDomains"
          type="number"
          min="0"
      /></label>
      <label class="check-field"
        ><input v-model="form.settings.fullHistory" type="checkbox" /> Full
        history</label
      >
      <label class="check-field"
        ><input v-model="form.settings.enableCidr24" type="checkbox" /> Enable
        CIDR /24</label
      >
      <label class="check-field"
        ><input
          v-model="form.settings.includeWeakFingerprints"
          type="checkbox"
        />
        Weak fingerprints</label
      >
    </div>
    <div v-else-if="tab === 'probe'" class="form-grid three">
      <label class="field"
        ><span>Priority rate</span
        ><input v-model.number="form.settings.priorityRate" type="number"
      /></label>
      <label class="field"
        ><span>Other rate</span
        ><input v-model.number="form.settings.otherRate" type="number"
      /></label>
      <label class="field"
        ><span>Workers</span
        ><input v-model.number="form.settings.workers" type="number"
      /></label>
      <label class="field"
        ><span>Timeout</span
        ><input v-model.number="form.settings.probeTimeout" type="number"
      /></label>
      <label class="field"
        ><span>Retries</span
        ><input v-model.number="form.settings.probeRetries" type="number"
      /></label>
      <label class="field"
        ><span>Content threshold</span
        ><input v-model.number="form.settings.contentThreshold" type="number"
      /></label>
      <label class="check-field"
        ><input v-model="form.settings.runRefine" type="checkbox" /> Run
        refine</label
      >
      <label class="check-field"
        ><input v-model="form.settings.runProbe" type="checkbox" /> Run
        probe</label
      >
      <label class="check-field"
        ><input v-model="form.settings.includeOther" type="checkbox" /> Include
        Q2/Q3</label
      >
      <label class="check-field"
        ><input v-model="form.settings.includeWeak" type="checkbox" /> Include
        Q1 weak</label
      >
    </div>
    <div v-else-if="tab === 'rules'" class="rule-grid">
      <label class="field"
        ><span>Gambling keywords（每行一个）</span
        ><textarea v-model="keywordText" rows="8"></textarea>
      </label>
      <label class="field"
        ><span>Porn keywords（每行一个）</span
        ><textarea v-model="pornText" rows="8"></textarea>
      </label>
      <label class="field"
        ><span>Negative/context keywords</span
        ><textarea v-model="negativeText" rows="8"></textarea>
      </label>
      <label class="check-field span-three"
        ><input
          v-model="form.settings.replaceDefaultContentRules"
          type="checkbox"
        />
        {{
          tr(
            "仅使用自定义规则（不叠加内置高置信规则）",
            "Use custom rules only (do not include built-in high-confidence rules)",
          )
        }}</label
      >
      <p class="helper span-three">
        {{
          tr(
            "规则会随任务生成独立 JSON 快照并由探测脚本加载；修改规则后缓存策略也会自动更新。",
            "Rules are snapshotted per job and loaded by the probe worker; changing them also invalidates the probe cache.",
          )
        }}
      </p>
    </div>
    <div v-else class="security-rule-settings">
      <div class="security-rule-intro">
        <Database :size="17" />
        <div>
          <strong>{{ tr("代码审计规则源", "Code audit rule sources") }}</strong
          ><span>{{
            tr(
              "地址或分支留空时自动使用系统内置源。同步仅写入本机版本化缓存，并与上一个本地版本比较。",
              "Blank repository or reference fields use the built-in source. Sync writes to a versioned local cache and compares it with the previous local version.",
            )
          }}</span>
        </div>
      </div>
      <article
        v-for="definition in ruleDefinitions"
        :key="definition.key"
        class="security-rule-card"
      >
        <header>
          <div class="rule-title">
            <GitBranch :size="16" />
            <div>
              <strong>{{ definition.name }}</strong
              ><span>{{ definition.description }}</span>
            </div>
          </div>
          <span
            class="rule-pack-status"
            :class="rulePack(definition.key)?.status || 'not_installed'"
            >{{ ruleStatus(rulePack(definition.key)) }}</span
          >
        </header>
        <div class="form-grid two">
          <label class="field"
            ><span>{{ tr("GitHub 仓库", "GitHub repository") }}</span
            ><input
              v-model="form.settings[definition.repositoryKey]"
              :placeholder="definition.builtinRepository"
            /><small v-if="!configuredValue(definition.repositoryKey)"
              >{{ tr("使用系统内置源", "Using built-in source") }}：{{
                definition.builtinRepository
              }}</small
            ></label
          >
          <label class="field"
            ><span>{{ tr("分支 / 标签", "Branch / tag") }}</span
            ><input
              v-model="form.settings[definition.referenceKey]"
              :placeholder="definition.builtinReference"
            /><small v-if="!configuredValue(definition.referenceKey)"
              >{{ tr("使用内置分支", "Using built-in reference") }}：{{
                definition.builtinReference
              }}</small
            ></label
          >
        </div>
        <div class="rule-version-row">
          <div>
            <span>{{ tr("同步前版本", "Previous version") }}</span
            ><code>{{ rulePack(definition.key)?.previousVersion || "—" }}</code>
          </div>
          <div>
            <span>{{ tr("当前本地版本", "Current local version") }}</span
            ><code>{{ rulePack(definition.key)?.version || "—" }}</code>
          </div>
          <div>
            <span>{{ tr("上次同步", "Last sync") }}</span
            ><time>{{ rulePack(definition.key)?.lastSyncAt || "—" }}</time>
          </div>
          <button
            type="button"
            class="button secondary compact"
            :disabled="!!ruleBusy"
            @click="syncRulePack(definition)"
          >
            <RefreshCw
              :size="14"
              :class="{ spinning: ruleBusy === definition.key }"
            />{{
              ruleBusy === definition.key
                ? tr("同步中", "Syncing")
                : tr("同步并比较", "Sync and compare")
            }}
          </button>
        </div>
        <div
          v-if="rulePack(definition.key)?.status === 'syncing'"
          class="rule-sync-progress"
        >
          <header>
            <span>{{ rulePack(definition.key)?.progressMessage || tr("正在准备", "Preparing") }}</span>
            <b>{{ rulePack(definition.key)?.progress || 0 }}%</b>
          </header>
          <div class="progress-track"><i :style="{ width: `${rulePack(definition.key)?.progress || 0}%` }"></i></div>
          <small>{{ rulePack(definition.key)?.progressStage || "prepare" }}</small>
        </div>
        <div
          v-if="rulePack(definition.key)?.status === 'ready'"
          class="rule-change-summary"
        >
          <span class="added"
            >+ {{ rulePack(definition.key)?.addedCount || 0 }}
            {{ tr("新增", "added") }}</span
          ><span class="modified"
            >~ {{ rulePack(definition.key)?.modifiedCount || 0 }}
            {{ tr("修改", "modified") }}</span
          ><span class="deleted"
            >- {{ rulePack(definition.key)?.deletedCount || 0 }}
            {{ tr("删除", "deleted") }}</span
          >
        </div>
        <div
          v-if="
            Array.isArray(rulePack(definition.key)?.changeSummary) &&
            rulePack(definition.key)!.changeSummary.length
          "
          class="rule-change-files"
        >
          <code
            v-for="change in rulePack(definition.key)!.changeSummary.slice(
              0,
              12,
            )"
            :key="`${change.status}-${change.path}`"
            ><b :class="change.status">{{
              change.status === "added"
                ? "+"
                : change.status === "deleted"
                  ? "-"
                  : "~"
            }}</b
            >{{ change.path }}</code
          ><small v-if="rulePack(definition.key)!.changeSummary.length > 12">{{
            tr(
              `另有 ${rulePack(definition.key)!.changeSummary.length - 12} 项，数据库最多保留 200 项差异明细。`,
              `${rulePack(definition.key)!.changeSummary.length - 12} more; up to 200 changes are retained.`,
            )
          }}</small>
        </div>
        <p v-if="rulePack(definition.key)?.error" class="rule-pack-error">
          {{ rulePack(definition.key)?.error }}
        </p>
      </article>
      <p v-if="ruleMessage" class="rule-pack-message">{{ ruleMessage }}</p>
    </div>
    <p v-if="error" class="form-error">{{ error }}</p>
    <template #footer>
      <button class="button ghost" @click="$emit('close')">
        {{ tr("取消", "Cancel") }}
      </button>
      <button class="button primary" :disabled="busy" @click="save">
        {{ busy ? tr("保存中…", "Saving…") : tr("保存配置", "Save profile") }}
      </button>
    </template>
  </ModalShell>
</template>
