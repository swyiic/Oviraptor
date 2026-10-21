<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import {
  Check,
  Database,
  Eye,
  EyeOff,
  GitBranch,
  KeyRound,
  Plus,
  RefreshCw,
  Trash2,
} from "@lucide/vue";
import { configurationApi } from "../api";
import { scanningRulePacksApi } from "../../sentinel/rulePacksApi";
import type { ConfigProfile } from "../types";
import type { ConfigurationCategory } from "../categoryTypes";
import type { SecurityRulePack } from "../../sentinel/rulePacksTypes";
import { useI18n } from "../../../i18n";
import { CONFIG_PROFILE_SETTING_DEFAULTS, createAgentModelProfile, type AgentModelProfile } from "../configProfileDefaults";
import { INDEPENDENT_POLICY_SECTIONS, cloneConfigProfileSettings, configPolicyPreviewSettings, configPolicyReference, modelConnectionSignature, modelProfilesRequiringTest, pickSectionSettings, redactedConfigPreview, resolveConfigProfileSection, rulePackSignature, type ConfigProfileSection, type RulePackDraft } from "../configProfileSections";

const props = defineProps<{
  profile?: ConfigProfile; section: ConfigProfileSection; category?: ConfigurationCategory;
  initialSettings?: Record<string, unknown>; modelDeployment?: "local" | "cloud"; proxyUrlContext?: string;
  locked?: boolean; readOnly?: boolean; saveLabel?: string;
  persist?: (settings: Record<string, unknown>) => Promise<void>;
}>();
const emit = defineEmits<{ saved: [id: number]; reload: []; forkDraft: [settings: Record<string, unknown>] }>();
const { tr } = useI18n();
let disposed = false;
onUnmounted(() => { disposed = true; });


let agentProfileSequence = 0;
function newAgentModelProfile(name = ""): AgentModelProfile {
  agentProfileSequence += 1;
  return createAgentModelProfile(name || tr("新模型", "New model"), agentProfileSequence);
}

const defaults = { ...CONFIG_PROFILE_SETTING_DEFAULTS, modelProfiles: [] as AgentModelProfile[] };
const storedSettings = cloneConfigProfileSettings(props.category?.settings ?? props.profile?.settings ?? {});
const draftSettings = cloneConfigProfileSettings(props.initialSettings ?? storedSettings);
const loadedCategoryReference = props.category?.reference ? { ...props.category.reference } : undefined;
// The configuration API supplies current profiles and discards retired settings.
function normalizedAgentProfiles(): AgentModelProfile[] {
  const source = Array.isArray(draftSettings.modelProfiles)
    ? draftSettings.modelProfiles
    : null;
  if (source) {
    return source
      .filter((value: any) => value && typeof value === "object")
      .map((value: any, index: number) => ({
        ...value,
        id: String(value.id || `agent-model-${index + 1}`),
        name: String(value.name || `${tr("模型", "Model")} ${index + 1}`),
        llm: String(value.llm || ""),
        apiBase: String(value.apiBase || ""),
        apiKey: String(value.apiKey || ""),
        localApiKey: String(value.localApiKey || ""),
        deployment: value.deployment === "local" ? "local" : "cloud",
      }));
  }
  return [];
}
const initialAgentProfiles = normalizedAgentProfiles();
const originalAgentProfiles = initialAgentProfiles.map(profile => ({ ...profile }));
function initialNativeAnalyzers() {
  const stored = draftSettings.nativeAnalyzers as Record<string, { image?: unknown }> | undefined;
  return {
    ...stored,
    semgrep: { ...stored?.semgrep, image: String(stored?.semgrep?.image ?? "") },
    codeql: { ...stored?.codeql, image: String(stored?.codeql?.image ?? "") },
  };
}
const form = reactive({
  settings: {
    ...defaults,
    ...draftSettings,
    ...(props.proxyUrlContext !== undefined ? { proxyUrl: props.proxyUrlContext } : {}),
    modelProfiles: initialAgentProfiles,
    nativeAnalyzers: initialNativeAnalyzers(),
    activeModelProfileId:
      String(draftSettings.activeModelProfileId || "") ||
      initialAgentProfiles[0]?.id ||
      "",
  },
});
const tab = ref<ConfigProfileSection>(resolveConfigProfileSection(props.section));
const existingProfile = computed(() => Boolean(props.category || (props.profile?.id && props.profile.id > 0)));
const policySource = computed(() => props.category ? loadedCategoryReference : configPolicyReference(storedSettings, tab.value));
const canAdoptPolicy = computed(() => !props.category && existingProfile.value && INDEPENDENT_POLICY_SECTIONS.includes(tab.value) && !policySource.value);
const policyPreview = ref<{ section: ConfigProfileSection; settings: Record<string, unknown> }>();
watch(tab, () => { policyPreview.value = undefined; });
watch(() => props.section, section => { tab.value = resolveConfigProfileSection(section); });
const busy = ref(false);
const error = ref("");
const sourceChanged = computed(() => JSON.stringify(pickSectionSettings(props.category?.settings ?? props.profile?.settings ?? {}, tab.value)) !== JSON.stringify(pickSectionSettings(storedSettings, tab.value)) || JSON.stringify(props.category ? props.category.reference ?? undefined : configPolicyReference(props.profile?.settings ?? {}, tab.value)) !== JSON.stringify(policySource.value));
const rulePacks = ref<SecurityRulePack[]>([]);
const ruleBusy = ref("");
const ruleMessage = ref("");
const ruleError = ref("");
const ruleLoading = ref(true);
const ruleDrafts = reactive<Record<string, RulePackDraft>>({});
const ruleBaselines = reactive<Record<string, string>>({});
const modelTestBusy = ref(false);
const modelTestStatus = ref<"idle" | "passed" | "failed">("idle");
const modelTestMessage = ref("");
const fofaTestBusy = ref(false);
const showFofaKey = ref(false);
const fofaTestStatus = ref<"idle" | "passed" | "failed">("idle");
const fofaTestMessage = ref("");
const modelTestKeys = reactive<Record<string, string>>({});
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
for (const definition of ruleDefinitions) {
  const draft = { repository: definition.builtinRepository, reference: definition.builtinReference, enabled: true };
  ruleDrafts[definition.key] = draft;
  ruleBaselines[definition.key] = rulePackSignature(draft);
}
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
const activeAgentProfile = computed(() =>
  form.settings.modelProfiles.find(
    (profile) => profile.id === form.settings.activeModelProfileId,
  ),
);
const localProfileActive = computed(
  () => (props.modelDeployment ?? activeAgentProfile.value?.deployment) === "local",
);
const cloudPolicyVisible = computed(
  () => props.modelDeployment ? props.modelDeployment === "cloud" : !activeAgentProfile.value || activeAgentProfile.value.deployment === "cloud",
);

function profileTestSignatureSource(profile: AgentModelProfile) {
  return modelConnectionSignature(profile);
}
function profileHasAcceptedTest(profile: AgentModelProfile) {
  const signature = profileTestSignatureSource(profile);
  return signature === modelTestKeys[profile.id];
}
async function testActiveAgentProfile() {
  const profile = activeAgentProfile.value;
  if (!profile) return;
  modelTestBusy.value = true;
  modelTestStatus.value = "idle";
  modelTestMessage.value = "";
  const signature = profileTestSignatureSource(profile);
  try {
    const result = await configurationApi.testModelProfile({
      llm: profile.llm.trim(),
      deployment: profile.deployment,
      apiBase: profile.apiBase.trim(),
      apiKey: profile.deployment === "local" ? profile.localApiKey : profile.apiKey,
    });
    if (!result.ok) throw new Error(result.message);
    if (signature !== profileTestSignatureSource(profile)) throw new Error(tr("测试期间模型连接已修改，请重新测试。", "The model connection changed during the test. Test it again."));
    modelTestKeys[profile.id] = signature;
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
  if (props.category && props.proxyUrlContext === undefined) {
    fofaTestStatus.value = "failed"; fofaTestMessage.value = tr("业务网络尚未读取，不能确认测试路由。请重新加载此分类。", "Business networking has not loaded, so the test route is unknown. Reload this category."); return;
  }
  fofaTestBusy.value = true;
  fofaTestStatus.value = "idle";
  fofaTestMessage.value = "";
  try {
    const result = await configurationApi.testFofaApi({
      key: String(form.settings.fofaKey || "").trim(),
      proxyUrl: props.category ? String(props.proxyUrlContext ?? "").trim() : String(form.settings.proxyUrl || "").trim(),
      email: String(form.settings.fofaEmail || "").trim(),
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

function activateAgentProfile(id: string) {
  form.settings.activeModelProfileId = id;
  syncActiveModelFields();
  const profile = activeAgentProfile.value;
  const accepted = profile ? profileHasAcceptedTest(profile) : false;
  modelTestStatus.value = accepted ? "passed" : "idle";
  modelTestMessage.value = accepted
    ? tr("该模型已通过连通性测试。", "This model passed the connectivity test.")
    : "";
}
function addAgentProfile() {
  const profile = newAgentModelProfile();
  form.settings.modelProfiles.push(profile);
  activateAgentProfile(profile.id);
}
function removeAgentProfile(id: string) {
  const index = form.settings.modelProfiles.findIndex(
    (profile) => profile.id === id,
  );
  if (index < 0) return;
  form.settings.modelProfiles.splice(index, 1);
  if (form.settings.activeModelProfileId === id) {
    form.settings.activeModelProfileId =
      form.settings.modelProfiles[Math.min(index, form.settings.modelProfiles.length - 1)]
        ?.id || "";
  }
  syncActiveModelFields();
  delete modelTestKeys[id];
  modelTestStatus.value =
    activeAgentProfile.value && profileHasAcceptedTest(activeAgentProfile.value)
      ? "passed"
      : "idle";
  modelTestMessage.value = "";
}

watch(
  () => activeAgentProfile.value?.deployment,
  (deployment) => {
    if (deployment !== "local" && form.settings.agentLocalFullPower) {
      form.settings.agentLocalFullPower = false;
    }
    if (deployment !== "local") form.settings.agentPromptAuditMode = "off";
  },
);
watch(
  () => (activeAgentProfile.value ? profileTestSignatureSource(activeAgentProfile.value) : ""),
  () => {
    const profile = activeAgentProfile.value;
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
/// §8.1: the flat neutral keys mirror the profile in use. The cloud key is never
/// mirrored into the local one, and vice versa, so a local endpoint cannot borrow
/// a credential that belongs to the cloud side.
function syncActiveModelFields() {
  const profile = activeAgentProfile.value;
  form.settings.modelDeployment = profile?.deployment === "local" ? "local" : "cloud";
  form.settings.modelApiBase = profile?.apiBase.trim() || "";
  form.settings.modelApiKey = profile?.deployment === "cloud" ? profile.apiKey.trim() : "";
  form.settings.localApiKey = profile?.deployment === "local" ? profile.localApiKey.trim() : "";
  (form.settings as Record<string, unknown>).modelName = profile?.llm.trim() || "";
}

function configuredValue(key: string) {
  const definition = ruleDefinitions.find(item => item.repositoryKey === key || item.referenceKey === key);
  if (!definition) return "";
  const draft = ruleDrafts[definition.key];
  return String(draft?.[definition.repositoryKey === key ? "repository" : "reference"] ?? "").trim();
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
  ruleLoading.value = true;
  ruleError.value = "";
  try {
    rulePacks.value = await scanningRulePacksApi.listSecurityRulePacks();
    for (const definition of ruleDefinitions) {
      const pack = rulePack(definition.key);
      // Refresh status without replacing unsaved global rule edits.
      if (!ruleDrafts[definition.key] || rulePackSignature(ruleDrafts[definition.key]) === ruleBaselines[definition.key]) {
        const draft = { repository: pack?.repository || definition.builtinRepository, reference: pack?.reference || definition.builtinReference, enabled: pack?.enabled ?? true };
        ruleDrafts[definition.key] = draft;
        ruleBaselines[definition.key] = pack ? rulePackSignature(draft) : "";
      }
    }
  } catch (reason) {
    ruleError.value = String(reason);
  } finally {
    ruleLoading.value = false;
  }
}
function rulePackChanged(key: string) {
  return Boolean(ruleDrafts[key] && rulePackSignature(ruleDrafts[key]) !== ruleBaselines[key]);
}
async function saveRulePack(definition: (typeof ruleDefinitions)[number]) {
  if (ruleBusy.value || !rulePackChanged(definition.key)) return;
  ruleBusy.value = definition.key;
  ruleError.value = ""; ruleMessage.value = "";
  const draft = { ...ruleDrafts[definition.key] };
  try {
    await scanningRulePacksApi.saveSecurityRulePack({ key: definition.key, name: definition.name, engine: definition.engine, ...draft });
    ruleBaselines[definition.key] = rulePackSignature(draft);
    await loadRulePacks();
    ruleMessage.value = tr("已保存全局规则源；分析器配置仍单独保存。需要更新本地规则时再点同步。", "Global rule source saved; analyzer settings save separately. Sync explicitly to update local rules.");
  } catch (reason) { ruleError.value = String(reason); }
  finally { ruleBusy.value = ""; }
}
async function syncRulePack(definition: (typeof ruleDefinitions)[number]) {
  const pack = rulePack(definition.key);
  if (ruleBusy.value || !pack || rulePackChanged(definition.key)) return;
  ruleBusy.value = definition.key;
  ruleError.value = "";
  ruleMessage.value = "";
  try {
    await scanningRulePacksApi.syncSecurityRulePack(pack.id);
    ruleMessage.value = `${definition.name} ${tr("已进入后台同步", "is syncing in the background")}`;
    for (let attempt = 0; attempt < 600; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 700));
      if (disposed) return;
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
    ruleError.value = String(reason);
  } finally {
    ruleBusy.value = "";
  }
}

function validatedDraft() {
    if (sourceChanged.value) throw new Error(tr("此分类已更新，请重新加载后保存；当前草稿尚未提交。", "This category changed. Reload before saving; this draft has not been submitted."));
    if (tab.value === "models" && !policySource.value) validateModelAdoption();
    if (tab.value === "models") syncActiveModelFields();
    if (tab.value === "securityRules") for (const engine of ["semgrep", "codeql"] as const) {
      const config = form.settings.nativeAnalyzers[engine];
      config.image = config.image.trim();
      if (config.image && !/^[^\s@]+@sha256:[0-9a-fA-F]{64}$/.test(config.image)) {
        throw new Error(tr(`${engine} 镜像必须固定到 @sha256: 加 64 位摘要；不能使用 latest 等可变标签。`, `${engine} requires an image pinned with @sha256: and a 64-character digest.`));
      }
    }
    if (tab.value === "models") for (const profile of modelProfilesRequiringTest(originalAgentProfiles, form.settings.modelProfiles)) {
      if (!profileHasAcceptedTest(profile)) {
        throw new Error(
          tr(
            `模型「${profile.name || profile.llm}」连接已修改，请先切到该模型并通过连通性测试。未修改模型时，其他分类保存无需测试。`,
            `The connection for ${profile.name || profile.llm} changed. Select it and pass a connectivity test. Unchanged models do not block other category saves.`,
          ),
        );
      }
    }
    return cloneConfigProfileSettings(configPolicyPreviewSettings(form.settings, tab.value));
}
function forkDraft() {
  if (busy.value || props.locked) return;
  error.value = "";
  try { emit("forkDraft", validatedDraft()); }
  catch (reason) { error.value = String(reason); }
}
async function save() {
  if (busy.value || props.locked || props.readOnly) return;
  busy.value = true; error.value = "";
  try {
    const settings = validatedDraft();
    if (props.category) {
      if (!props.persist) throw new Error("configuration_category_editor_not_connected");
      await props.persist(settings);
      return;
    }
    if (!existingProfile.value) throw new Error(tr("此分类尚未加载，请重新加载后保存。", "This category has not loaded. Reload before saving."));
    const input = { profileId: props.profile!.id, section: tab.value, settings, expectedSettings: pickSectionSettings(storedSettings, tab.value), ...(policySource.value ? { expectedReference: policySource.value } : {}) };
    const id = policySource.value ? await configurationApi.saveProfileSection(input) : await configurationApi.adoptProfilePolicy(input);
    if (!disposed) emit("saved", id);
  } catch (reason) { error.value = String(reason); }
  finally { busy.value = false; }
}

function validateModelAdoption() {
  if (form.settings.modelProfiles.some(profile => !profile.apiBase.trim() || (profile.deployment === "cloud" && !profile.apiKey.trim()))) {
    throw new Error(tr("独立扫描模型需要明确的 API 地址和对应凭据；不会继承环境变量或聊天 OAuth。", "Independent scan models require an explicit API address and credentials. They do not inherit environment variables or chat OAuth."));
  }
}
function previewPolicy() {
  if (busy.value || !canAdoptPolicy.value) return;
  error.value = "";
  if (tab.value === "models") {
    try { validateModelAdoption(); } catch (reason) { error.value = String(reason); return; }
    syncActiveModelFields();
  }
  policyPreview.value = { section: tab.value, settings: cloneConfigProfileSettings(configPolicyPreviewSettings(form.settings, tab.value)) };
}

async function adoptPolicy() {
  const preview = policyPreview.value;
  if (busy.value || !preview || !canAdoptPolicy.value) return;
  if (preview.section !== tab.value || JSON.stringify(preview.settings) !== JSON.stringify(configPolicyPreviewSettings(form.settings, tab.value))) {
    error.value = tr("预览后配置已修改，请重新预览再创建独立策略。", "Settings changed after preview. Preview again before creating this policy.");
    policyPreview.value = undefined; return;
  }
  busy.value = true; error.value = "";
  try {
    if (preview.section === "models") for (const profile of modelProfilesRequiringTest(originalAgentProfiles, form.settings.modelProfiles)) {
      if (!profileHasAcceptedTest(profile)) throw new Error(tr("模型连接已修改，请先通过连通性测试再拆分。", "Model connection changed. Pass its connectivity test before splitting."));
    }
    const id = await configurationApi.adoptProfilePolicy({ profileId: props.profile!.id, section: preview.section, settings: preview.settings, expectedSettings: pickSectionSettings(storedSettings, preview.section) });
    if (!disposed) emit("saved", id);
  } catch (reason) { error.value = String(reason); }
  finally { busy.value = false; }
}
onMounted(() => { if (tab.value === "securityRules") void loadRulePacks(); });
</script>

<template>
  <section class="profile-inline-editor" data-testid="profile-section-editor" :data-section="section">
    <p v-if="sourceChanged" class="form-error">{{ tr('此分类的已保存配置已更新。草稿保留；重新加载后可编辑最新版本。', 'Saved settings changed while this draft was open. Your draft is retained; reload to edit the latest revision.') }} <button type="button" class="button ghost compact" :disabled="busy" @click="$emit('reload')">{{ tr('重新加载此分类', 'Reload this category') }}</button></p>
    <section v-if="!category && existingProfile && INDEPENDENT_POLICY_SECTIONS.includes(tab)" class="policy-source panel" data-testid="configuration-policy-source">
      <p v-if="policySource">{{ tr(`当前分类已独立保存，第 ${policySource.revision} 版。保存会创建新版本。`, `This category has independent storage at revision ${policySource.revision}. Saving creates a new revision.`) }}</p>
      <template v-else>
        <p>{{ tr('当前分类读取历史兼容来源。首次保存会独立存储此分类；其他分类及历史原文保留。预览是可选操作。扫描 API 账号与聊天 Grok OAuth 分别管理。', 'This category currently reads a historical source. Its first save creates independent storage for this category only, preserving other categories and the source. Preview is optional. Scan API accounts and chat Grok OAuth are managed separately.') }}</p>
        <button class="button ghost compact" type="button" :disabled="busy" @click="previewPolicy">{{ tr('预览拆分', 'Preview split') }}</button>
        <div v-if="policyPreview" class="policy-preview">
          <p>{{ tr('确认后，此分类读取独立配置；凭据单独保存，预览已隐藏密钥，原方案内容保留。', 'After confirmation this category uses independent storage. Credentials are stored separately and hidden here; the original profile is retained.') }}</p>
          <pre>{{ JSON.stringify(redactedConfigPreview(configPolicyPreviewSettings(policyPreview.settings, policyPreview.section)), null, 2) }}</pre>
          <button class="button primary compact" type="button" :disabled="busy" @click="adoptPolicy">{{ tr('确认创建独立配置', 'Create independent configuration') }}</button>
          <button class="button ghost compact" type="button" :disabled="busy" @click="policyPreview=undefined">{{ tr('取消拆分', 'Cancel split') }}</button>
        </div>
      </template>
    </section>
    <p v-if="error" class="form-error profile-error-top">{{ error }}</p>
    <fieldset class="category-fields" :disabled="Boolean(locked)">
    <div v-if="tab === 'models'" class="account-settings" data-testid="profile-section-models">
      <div class="account-settings-intro">
        <KeyRound :size="17" />
        <div>
          <strong>{{ tr("扫描 / Nest 模型", "Scan / Nest models") }}</strong>
          <span>{{
            tr(
              "此模型由 Rust 扫描 / Nest 和旧版对话使用。新扫描读取此分类的默认配置；编辑其他配置不会切换默认。聊天 Agent 的 OAuth / API 单独管理。",
              "Rust scanning / Nest and legacy chat use this model. New scans read this category’s default. Editing another configuration does not switch that default. Chat Agent OAuth / API settings are managed separately.",
            )
          }}</span>
        </div>
      </div>

      <p class="helper model-test-cost">{{ tr('修改模型连接后，保存前需要通过连通性测试。测试会调用对应模型服务，可能产生少量费用；未修改的模型不会阻挡其他分类保存。', 'A changed model connection requires a connectivity test before saving. The test calls that model service and may incur a small charge. Unchanged models do not block other categories.') }}</p>
      <section class="account-provider-section agent-model-section">
        <header>
          <div>
            <strong>{{ tr("扫描 / Nest 模型连接", "Scan / Nest model connection") }}</strong>
            <span>{{
              tr(
                "启用标记选择此分类使用的扫描模型。保存仅更新当前来源的模型配置，不会更改聊天登录或其他分类。",
                "The active marker selects this category’s scan model. Saving updates only its current source, leaving chat sign-in and other categories unchanged.",
              )
            }}</span>
          </div>
          <button
            class="button ghost compact"
            type="button"
            @click="addAgentProfile"
          >
            <Plus :size="14" />{{ tr("添加模型", "Add model") }}
          </button>
        </header>
        <p v-if="error" class="form-error model-error-inline">{{ error }}</p>

        <div v-if="form.settings.modelProfiles.length" class="model-switch-list">
          <button
            v-for="profile in form.settings.modelProfiles"
            :key="profile.id"
            type="button"
            :class="{ active: profile.id === form.settings.activeModelProfileId }"
            :aria-pressed="profile.id === form.settings.activeModelProfileId"
            @click="activateAgentProfile(profile.id)"
          >
            <span>
              <strong>{{ profile.name || tr("未命名模型", "Unnamed model") }}</strong>
              <small>{{ profile.deployment === "local" ? tr("本地自建", "Self-hosted") : tr("云端", "Cloud") }} · {{ profile.llm || tr("尚未填写模型 ID", "Model ID not set") }}</small>
            </span>
            <em v-if="profile.id === form.settings.activeModelProfileId">
              <Check :size="12" />{{ tr("当前启用", "Active") }}
            </em>
            <em v-else>{{ tr("切换", "Switch") }}</em>
          </button>
        </div>
        <div v-else class="model-profile-empty">
          {{ tr("还没有模型配置。添加后即可在这里切换。", "No model profiles yet. Add one to enable switching.") }}
        </div>

        <div v-if="activeAgentProfile" class="model-profile-editor">
          <header>
            <div>
              <strong>{{ tr("编辑当前模型", "Edit active model") }}</strong>
              <span>{{ activeAgentProfile.name }}</span>
            </div>
            <div class="model-editor-actions">
              <button
                class="button danger compact model-delete-button"
                type="button"
                :title="tr('删除此模型', 'Delete this model')"
                @click="removeAgentProfile(activeAgentProfile.id)"
              >
                <Trash2 :size="14" />{{ tr("删除模型", "Delete model") }}
              </button>
              <button
                class="button secondary compact model-test-button"
                type="button"
                :disabled="modelTestBusy"
                @click="testActiveAgentProfile"
              >
                <RefreshCw :size="14" :class="{ spinning: modelTestBusy }" />
                {{ modelTestBusy ? tr("测试中", "Testing") : tr("测试模型", "Test model") }}
              </button>
            </div>
          </header>
          <div class="form-grid two">
            <label class="field"
              ><span>{{ tr("显示名称", "Display name") }}</span
              ><input v-model="activeAgentProfile.name" placeholder="DeepSeek V4"
            /></label>
            <label class="field"
              ><span>{{ activeAgentProfile.deployment === "local" ? tr("本地模型 ID", "Local model ID") : tr("云端模型 ID", "Cloud model ID") }}</span
              ><input v-model="activeAgentProfile.llm" placeholder="deepseek/deepseek-v4-pro"
            /></label>
            <label class="field span-two"
              ><span>{{ tr("部署类型", "Deployment") }}</span
              ><select
                v-model="activeAgentProfile.deployment"
              >
                <option value="cloud">{{ tr("云端 API", "Cloud API") }}</option>
                <option value="local">{{ tr("本地自建 OpenAI 兼容服务", "Self-hosted OpenAI-compatible") }}</option>
              </select>
              <small>{{
                activeAgentProfile.deployment === "local"
                  ? tr("只显示本地服务地址和模型 ID；云端凭据与费用策略不会参与本地任务。", "Only the local endpoint and model ID are shown; cloud credentials and cost policy do not apply to local tasks.")
                  : tr("只显示云端 API 凭据和云端发现预算；本地算力、Hook 与火力全开选项会隐藏。", "Only cloud API credentials and cloud discovery budgets are shown; local compute, hook, and full-power options are hidden.")
              }}</small>
            </label>
            <label v-if="activeAgentProfile.deployment === 'cloud'" class="field span-two"
              ><span>{{ tr("云端 API Base URL", "Cloud API Base URL") }}</span
              ><input
                v-model="activeAgentProfile.apiBase"
                placeholder="https://api.example.com/v1"
            /></label>
            <label v-if="activeAgentProfile.deployment === 'cloud'" class="field span-two"
              ><span>{{ tr("云端 API Key", "Cloud API key") }}</span
              ><input
                v-model="activeAgentProfile.apiKey"
                type="password"
                autocomplete="new-password"
                :placeholder="tr('请输入云端模型 API Key', 'Enter the cloud model API key')"
            /></label>
            <label v-if="activeAgentProfile.deployment === 'local'" class="field span-two"
              ><span>{{ tr("本地服务地址", "Local service endpoint") }}</span
              ><input v-model="activeAgentProfile.apiBase" placeholder="http://127.0.0.1:11434/v1"
            /></label>
            <label v-if="activeAgentProfile.deployment === 'local'" class="field span-two"
              ><span>{{ tr("本地服务 API Key（可选）", "Local service API key (optional)") }}</span
              ><input
                v-model="activeAgentProfile.localApiKey"
                type="password"
                autocomplete="new-password"
                :placeholder="tr('无鉴权服务保持留空', 'Leave blank when authentication is disabled')"
            /></label>
            <small v-if="activeAgentProfile.deployment === 'local'" class="helper span-two">{{ tr("留空时只向本机服务发送占位凭据 local；不会继承云端 API Key、LLM_API_KEY 或其他全局凭据。上下文窗口、KV Cache 和推理线程仍由本地服务管理。", "When blank, only the local placeholder credential is sent. Cloud API keys, LLM_API_KEY, and other global credentials are never inherited. Context size, KV cache, and inference threads remain controlled by the local service.") }}</small>
          </div>
          <p v-if="modelTestMessage" class="model-test-message" :class="modelTestStatus">
            <Check v-if="modelTestStatus === 'passed'" :size="13" />{{ modelTestMessage }}
          </p>
        </div>
      </section>
    </div>
    <div v-else-if="tab === 'assetApis'" class="account-settings" data-testid="profile-section-asset-apis">
      <p class="helper">{{ tr('FOFA 供使用此来源的资产采集任务使用。代理在「业务网络与代理」中配置。', 'FOFA is used by asset jobs using this source. Configure its proxy under Business network & proxies.') }}</p>
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
          <label class="field span-two"><span>{{tr('FOFA 账户 / Email','FOFA account / email')}}</span><input v-model="form.settings.fofaEmail" type="text" autocomplete="username" :placeholder="tr('账户标识，历史空值保持兼容','Account identity; historical empty values remain supported')" /></label>
          <label class="field"
            ><span>FOFA key</span
            ><div class="secret-input"><input
              v-model="form.settings.fofaKey"
              :type="showFofaKey ? 'text' : 'password'"
              autocomplete="new-password"
              :placeholder="tr('仅保存在本机应用数据库', 'Stored only in the local app database')"
          /><button class="button ghost compact" type="button" :aria-label="showFofaKey ? tr('隐藏 FOFA Key','Hide FOFA key') : tr('显示 FOFA Key','Show FOFA key')" :aria-pressed="showFofaKey" @click="showFofaKey=!showFofaKey"><EyeOff v-if="showFofaKey" :size="16"/><Eye v-else :size="16"/></button></div></label>
          <label class="field span-two"
            ><span>{{ tr("旧版 FOFA config 路径（Key 留空时使用）", "Legacy FOFA config path (used when key is blank)") }}</span
            ><input v-model="form.settings.configPath"
          /></label>
        </div>
        <p v-if="fofaTestMessage" class="model-test-message" :class="fofaTestStatus">
          <Check v-if="fofaTestStatus === 'passed'" :size="13" />{{ fofaTestMessage }}
        </p>
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
    <div v-else-if="tab === 'network'" class="form-grid two" data-testid="profile-section-network">
      <p class="helper span-two">{{ tr('此分类的默认网络参数供新的 FOFA 与 Rust 扫描任务使用。聊天 Agent 的 OAuth / API 连接在聊天模型的网络设置中单独配置。运行依赖的检测与安装在「运行环境」。', 'These network settings serve FOFA and Rust scans. Configure chat Agent OAuth / API networking separately under chat-model settings. Check dependencies under Environment.') }}</p>
      <label class="field"
        ><span>{{ tr("FOFA API 代理", "FOFA API proxy") }}</span
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
          ><input v-model="form.settings.agentProxyEnabled" type="checkbox" />
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
              "仅接受你拥有或明确获准使用的代理。Rust Web 扫描会按子批次轮换；此代理池不影响聊天 Agent 的 OAuth / API 连接。",
              "Only use proxies you own or are explicitly allowed to use. Rust Web scans rotate through the list; this pool does not affect chat Agent OAuth / API connections.",
            )
          }}
        </p>
      </div>
    </div>
    <div v-else-if="tab === 'agent'" class="agent-policy-settings">
      <p class="helper">{{ tr('此分类的默认策略用于新启动的 Rust 扫描 / Nest 调查，不控制聊天 Agent，也不改变远程 Worker 连接。', 'These settings apply to new Rust scans / Nest investigations using this category’s default. Chat Agent and remote Worker connections have separate settings.') }}</p>
      <div class="form-grid two">
        <label class="field"><span>{{ tr('扫描前端上下文模式', 'Scan frontend context mode') }}</span><select v-model="form.settings.agentFrontendPacketMode"><option value="balanced">{{ tr('按预算整理', 'Use configured budget') }}</option><option value="compact">{{ tr('精简（6 KiB）', 'Compact (6 KiB)') }}</option><option value="custom">{{ tr('自定义预算', 'Custom budget') }}</option></select></label>
        <label class="field"><span>{{ tr('扫描前端上下文预算（KiB）', 'Scan frontend context budget (KiB)') }}</span><input v-model.number="form.settings.agentFrontendPacketBudgetKb" type="number" min="4" max="64" :disabled="form.settings.agentFrontendPacketMode === 'compact'" /><small>{{ tr('范围 4–64 KiB；本地模型另受 12 KiB 上限约束。', 'Range: 4–64 KiB; local models also have a 12 KiB ceiling.') }}</small></label>
      </div>
      <div class="skill-format-guide">
        <strong>{{ tr("Rust Nest 扫描策略", "Rust Nest scan policy") }}</strong
        ><span>{{
          localProfileActive
            ? tr("当前使用本地模型：云端费用和请求预算已隐藏，运行时会根据模型参数规模自动限制并发、输出长度和首轮等待时间。", "A local model is active. Cloud cost and request budgets are hidden; runtime concurrency, output length, and first-turn wait are adapted to model size automatically.")
            : tr("当前使用云端 API：Native Root 协调专家任务，授权与预算由运行时校验；WAF、验证码与持续限流仍会停止目标请求。", "A cloud API is active. Native Root coordinates specialist tasks with runtime authorization and budget checks; WAF, CAPTCHA, and sustained rate limiting stop target requests.")
        }}</span>
      </div>
      <section class="agent-packet-policy">
        <div>
          <strong>{{ tr("SRC 专项能力已内置", "SRC specialist capabilities are built in") }}</strong>
          <span>{{ tr("原始 HTTP、有界竞争、按契约受控写入、攻击链关联和每任务 HTTP OAST 会自动准备，不再需要额外开关。目标无法回连当前工作站时，任务会只把 OAST 类别标记为网络不可达。", "Raw HTTP, bounded races, contract-gated writes, attack-chain correlation, and per-task HTTP OAST are prepared automatically. If the target cannot route back to this workstation, only OAST-dependent checks are marked unreachable.") }}</span>
        </div>
        <p class="helper">{{ tr("内置适配器由原生运行时执行；写请求仍必须包含清理、回滚、次数上限和业务不变量，不可逆操作继续禁止。", "The adapter uses the native runtime. Write contracts still require cleanup, rollback, attempt limits, and a business invariant; irreversible actions remain disabled.") }}</p>
      </section>
      <div v-if="localProfileActive" class="agent-governance-grid">
        <article class="agent-governance-card">
          <div>
            <strong>{{ tr("本地模型火力全开", "Local model full power") }}</strong>
            <span>{{ tr("仅当当前启用模型标记为“本地自建”时生效。", "Only applies when the active model is marked self-hosted.") }}</span>
          </div>
          <label class="switch-control">
            <input
              v-model="form.settings.agentLocalFullPower"
              type="checkbox"
              :disabled="!activeAgentProfile || activeAgentProfile.deployment !== 'local'"
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
        <article class="agent-governance-card">
          <div>
            <strong>{{ tr("本地模型 Hook 与提示词审计", "Local model hook and prompt audit") }}</strong>
            <span>{{ tr("Token 始终统计；这里控制新任务保存多少请求内容。", "Tokens are always counted; this controls how much request content new tasks retain.") }}</span>
          </div>
          <label class="field">
            <select
              v-model="form.settings.agentPromptAuditMode"
            >
              <option value="off">{{ tr("仅统计 Token", "Token counts only") }}</option>
              <option value="metadata">{{ tr("请求元数据与哈希", "Request metadata and hashes") }}</option>
              <option value="full">{{ tr("保存本机完整模型请求", "Store full model requests locally") }}</option>
            </select>
          </label>
          <p>{{
            tr(
              "本地自建模型通过回环 Hook 采集。全文档位会在本机记录原生 Agent 最终组装的 system / developer / user / tool schema、请求内容与响应摘要，仅对超长字段做截断；云模型仍只保留 Oviraptor 生成的 instruction。",
              "Self-hosted models are captured through a loopback hook. Full mode stores the native Agent's assembled system, developer, user, tool schema, request content, and response summary on this device, truncating only oversized fields. Cloud models retain only the Oviraptor-generated instruction.",
            )
          }}</p>
        </article>
      </div>
      <section v-if="localProfileActive" class="agent-packet-policy local-model-policy-note">
        <div><strong>{{ tr("本地模型自动资源策略", "Automatic local-model resource policy") }}</strong><span>{{ tr("从模型名称识别参数规模：20B–59B 严格串行、单次最多输出 4096 Token、首轮最长等待 1200 秒；60B 以上最多输出 3072 Token、等待 1800 秒；13B 以下仅在火力全开时允许最多 2 个上游请求。停止或暂停任务会立即断开仍在执行的本地推理，避免旧任务继续占用 CPU。", "Model size is inferred from its name: 20B–59B runs serially with at most 4,096 output tokens and a 1,200-second first-turn window; 60B+ uses 3,072 tokens and 1,800 seconds; below 13B allows at most two upstream requests only in full-power mode. Stopping or pausing a task immediately disconnects local inference so old tasks cannot keep consuming CPU.") }}</span></div>
        <p class="helper">{{ tr("模型服务自身的上下文窗口、KV Cache、量化方式和 CPU/GPU 线程仍由 MLX、LM Studio 或 Ollama 配置；Oviraptor 负责阻止并发放大和孤儿推理，不会偷偷把 Quick / Standard 改成 Deep。", "The model server still owns context length, KV cache, quantization, and CPU/GPU threads in MLX, LM Studio, or Ollama. Oviraptor prevents concurrency amplification and orphan inference, and never silently promotes Quick or Standard to Deep.") }}</p>
      </section>

      <fieldset v-if="cloudPolicyVisible" class="agent-policy-controls">
      <div class="form-grid two">
        <label class="field"
          ><span>{{ tr("无进展熔断轮次", "No-progress request limit") }}</span
          ><input v-model.number="form.settings.agentNoToolTurnLimit" type="number" min="1" max="100"
        /></label>
      </div>
      <div class="agent-policy-table">
        <div class="agent-policy-head">
          <span>{{ tr("模式", "Mode") }}</span>
          <span>{{ tr("起始评分", "Score") }}</span>
          <span>{{ tr("请求软预算", "Requests") }}</span>
          <span>{{ tr("超时（秒）", "Timeout (s)") }}</span>
          <span>{{ tr("Token 软预算", "Soft token budget") }}</span>
        </div>
        <div>
          <strong>Quick</strong>
          <input v-model.number="form.settings.agentQuickScore" type="number" min="1" max="90" />
          <input v-model.number="form.settings.agentQuickRequestLimit" type="number" min="1" max="100" />
          <input v-model.number="form.settings.agentQuickTimeout" type="number" min="30" />
          <input v-model.number="form.settings.agentQuickTokenLimit" type="number" min="0" step="10000" />
        </div>
        <div>
          <strong>Standard</strong>
          <input v-model.number="form.settings.agentStandardScore" type="number" min="2" max="95" />
          <input v-model.number="form.settings.agentStandardRequestLimit" type="number" min="1" max="200" />
          <input v-model.number="form.settings.agentStandardTimeout" type="number" min="60" />
          <input v-model.number="form.settings.agentStandardTokenLimit" type="number" min="0" step="10000" />
        </div>
        <div>
          <strong>Deep</strong>
          <input v-model.number="form.settings.agentDeepScore" type="number" min="3" max="100" />
          <input v-model.number="form.settings.agentDeepRequestLimit" type="number" min="1" max="300" />
          <input v-model.number="form.settings.agentDeepTimeout" type="number" min="120" />
          <input v-model.number="form.settings.agentDeepTokenLimit" type="number" min="0" step="10000" />
        </div>
      </div>
      <p class="helper">
        {{
          tr(
            "这里配置的是起始软预算：仍有新端点、响应差异或验证证据时会继续，连续无增量后才收口；Oviraptor 另有按模式和本地/云端区分的绝对硬上限。只有确认 WAF、验证码、机器人挑战或持续限流才进入熔断区。",
            "This is the initial soft budget: work continues while new endpoints, response differences, or verification evidence are appearing, and closes only after consecutive no-progress turns. Oviraptor separately enforces deployment-aware hard ceilings. Only confirmed WAF, CAPTCHA, bot challenge, or sustained rate limiting enters the fuse zone.",
          )
        }}
      </p>
      </fieldset>
    </div>
    <div v-else-if="tab === 'collection'" class="form-grid three">
      <label class="check-field span-two"><input v-model="form.settings.precisionFirst" type="checkbox"/>高准确率优先：域名范围先查，名称/正文/证书组织/ASN 等宽泛查询最多 1 页、每页最多 100 条</label>
      <p class="helper span-two">会员等级不等于全部语法/字段权限。F 点、API 查询额度和导出条数分别计算；额度不足立即停止。开启限制后的采集会明确标为覆盖受限，不保证查全。</p>
      <label class="field"
        ><span>FOFA profile</span
        ><select v-model="form.settings.fofaProfile">
          <option value="professional">旧版高级 / 专业字段（vip_level=2 或 12，实际权限由接口判定）</option>
          <option>business</option>
          <option>basic</option>
          <option value="auto">保守自动（不请求商业/企业专属字段）</option>
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
      <label class="check-field"
        ><input v-model="form.settings.fullHistory" type="checkbox" /> Full
        history</label
      >
      <label class="check-field"
        ><input v-model="form.settings.enableCidr24" type="checkbox" /> Enable
        CIDR /24</label
      >
      <details class="category-advanced span-three"><summary>{{tr('采集高级设置','Advanced collection settings')}}</summary><label class="check-field"><input v-model="form.settings.collectionCache" type="checkbox" />{{tr('允许复用采集缓存','Reuse collection cache')}}</label></details>
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
        ><span>{{tr('探测并发（1–256）','Probe workers (1–256)')}}</span
        ><input v-model.number="form.settings.workers" type="number" min="1" max="256"
      /></label>
      <label class="field"
        ><span>{{tr('单次请求超时（秒）','Request timeout (seconds)')}}</span
        ><input v-model.number="form.settings.probeTimeout" type="number"
      /></label>
      <label class="field"><span>{{tr('每个 URL 总时间预算（秒）','Total network budget per URL (seconds)')}}</span><input v-model.number="form.settings.probeTotalTimeout" type="number" min="1" max="60" /><small>{{tr('连接、重定向、响应读取及重试共用；超时结束本轮并保留记录。调度排队和响应归并计算另计。','Shared by connection, redirects, body reads and retries. Timeout stops the current probe and retains its record. Scheduling and response grouping are separate.')}}</small></label>
      <label class="field"
        ><span>Retries</span
        ><input v-model.number="form.settings.probeRetries" type="number"
      /></label>
      <label class="check-field"
        ><input :checked="form.settings.runRefine || form.settings.runProbe" :disabled="form.settings.runProbe" type="checkbox" @change="form.settings.runRefine = ($event.target as HTMLInputElement).checked" /> Run
        refine</label
      >
      <label class="check-field"
        ><input v-model="form.settings.runProbe" type="checkbox" /> Run
        probe</label
      >
      <p v-if="form.settings.runProbe" class="helper">{{ tr('资产探测依赖 refine；启用探测时会一并执行 refine。', 'Asset probing requires refine; enabling probing also runs refine.') }}</p>
      <label class="check-field"
        ><input v-model="form.settings.includeOther" type="checkbox" /> Include
        Q2/Q3</label
      >
      <label class="check-field"
        ><input v-model="form.settings.includeWeak" type="checkbox" /> Include
        Q1 weak</label
      >
      <details class="category-advanced span-three"><summary>{{tr('网络与响应高级设置','Advanced networking and response settings')}}</summary><div class="form-grid two">
        <label class="field"><span>{{tr('响应体读取上限（字节）','Maximum response body (bytes)')}}</span><input v-model.number="form.settings.maxBodyBytes" type="number" min="4096" max="1048576" step="4096" /><small>{{tr('范围 4096–1048576；较大响应增加内存与网络开销。','Range 4096–1048576; larger bodies use more memory and bandwidth.')}}</small></label>
        <label class="field"><span>{{tr('同一主机请求间隔（秒）','Per-host request interval (seconds)')}}</span><input v-model.number="form.settings.perHostInterval" type="number" min="0" max="3600" step="0.1" /></label>
        <label class="check-field"><input v-model="form.settings.allowPrivate" type="checkbox" />{{tr('允许探测私有网络地址','Allow probes to private-network addresses')}}</label>
        <label class="check-field"><input v-model="form.settings.strictTls" type="checkbox" />{{tr('严格校验 TLS 证书','Strictly verify TLS certificates')}}</label>
        <label class="check-field"><input v-model="form.settings.schemeFallback" type="checkbox" />{{tr('允许 HTTP / HTTPS 协议回退','Allow HTTP / HTTPS scheme fallback')}}</label>
      </div><p class="helper">{{tr('显示当前历史值；保存不会自动切换私网或 TLS 策略。允许私网会扩大请求范围；关闭严格 TLS 会接受无效证书。请明确核对后修改。','Current historical values are shown. Saving does not automatically switch private-network or TLS policy. Private-network access expands the target range; disabling strict TLS accepts invalid certificates. Review these choices explicitly.')}}</p></details>
    </div>
    <div v-else-if="tab === 'rules'" class="rule-grid">
      <p class="helper span-two">{{tr('新资产任务还会叠加资产模块中启用的全局标题与内容规则；任务启动时冻结规则，恢复任务保留原版本。清空此处关键词不会关闭那些全局规则。','New asset jobs also include enabled global title and content rules from Assets. Rules are frozen at job start and retained on resume; clearing keywords here does not disable those global rules.')}}</p>
      <label class="field"><span>{{ tr('内容分类阈值', 'Content classification threshold') }}</span><input v-model.number="form.settings.contentThreshold" type="number" /></label>
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
    <div v-else class="security-rule-settings" data-testid="profile-section-security-rules">
      <p class="helper">{{ tr('分析器镜像是此分类的独立配置，使用底部「保存当前分类」。下方规则包属于整台机器，只有各自的「保存规则源」会修改全局配置；同步是另一个显式动作。', 'Analyzer images are independent settings in this category. Use Save this category below. Rule packs belong to this machine: only Save rule source changes their global configuration. Sync is a separate action.') }}</p>
      <p v-if="ruleError" class="form-error" role="alert">{{ ruleError }} <button type="button" class="text-button" @click="loadRulePacks">{{ tr('重试读取规则包', 'Retry reading rule packs') }}</button></p>
      <p v-if="ruleLoading" class="helper" role="status">{{ tr('正在读取机器规则包…', 'Reading machine rule packs…') }}</p>
      <section class="security-rule-card">
        <header><strong>{{ tr('Native 分析器镜像', 'Native analyzer images') }}</strong></header>
        <p class="helper">{{ tr('仅用于源码 / 灰盒 / CI 分析。填写已批准且已加载到本机 Docker 的固定摘要镜像；应用不自动下载镜像。Web URL 调查不要求这些镜像。', 'For source, greybox and CI analysis only. Use approved, digest-pinned images already loaded into local Docker. Images are never downloaded automatically; Web URL tasks do not require them.') }}</p>
        <div class="form-grid two">
          <label v-for="engine in (['semgrep', 'codeql'] as const)" :key="engine" class="field span-two">
            <span>{{ engine }} · image@sha256</span>
            <input v-model="form.settings.nativeAnalyzers[engine].image" :aria-label="`${engine} pinned image`" placeholder="registry.example/analyzer@sha256:…" spellcheck="false" />
          </label>
        </div>
        <p class="helper">{{ tr('还需要启用下方对应的本地规则包。留空、规则缺失或镜像不可用会产生覆盖缺口，不表示扫描通过。', 'Enable the corresponding local rule pack below. Missing configuration, rules or images produce coverage gaps, not a passing scan.') }}</p>
      </section>
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
              v-model="ruleDrafts[definition.key].repository"
              :disabled="ruleLoading || !!ruleBusy"
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
              v-model="ruleDrafts[definition.key].reference"
              :disabled="ruleLoading || !!ruleBusy"
              :placeholder="definition.builtinReference"
            /><small v-if="!configuredValue(definition.referenceKey)"
              >{{ tr("使用内置分支", "Using built-in reference") }}：{{
                definition.builtinReference
              }}</small
            ></label
          >
        </div>
        <div class="rule-version-row">
          <label class="check-field"><input v-model="ruleDrafts[definition.key].enabled" type="checkbox" :disabled="ruleLoading || !!ruleBusy" />{{ tr('在本机启用此规则包', 'Enable this rule pack on this machine') }}</label>
          <button type="button" class="button primary compact" :disabled="ruleLoading || !!ruleBusy || !rulePackChanged(definition.key)" @click="saveRulePack(definition)">{{ tr('保存规则源', 'Save rule source') }}</button>
          <small v-if="rulePackChanged(definition.key)" class="helper">{{ tr('规则源有未保存更改；保存当前分类不会应用这些更改。', 'Rule source has unsaved changes; Save this category does not apply them.') }}</small>
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
            :disabled="ruleLoading || !!ruleBusy || !rulePack(definition.key) || rulePackChanged(definition.key)"
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
    </fieldset>
    <footer class="profile-section-actions">
      <span>{{ tr('只保存此分类；其他分类、聊天登录和全局规则源分别管理。', 'Saves only this category. Other categories, chat sign-in and global rule sources are managed separately.') }}</span>
      <button v-if="category" class="button ghost" :disabled="busy || locked || sourceChanged" @click="forkDraft">{{tr('另存为新配置','Save as new configuration')}}</button>
      <button class="button primary" :disabled="busy || locked || readOnly || !existingProfile || sourceChanged" @click="save">
        {{ busy ? tr("保存中…", "Saving…") : saveLabel || (policySource ? tr('保存当前分类', 'Save this category') : tr('保存为独立配置', 'Save independently')) }}
      </button>
    </footer>
  </section>
</template>

<style scoped>
.secret-input{display:flex;align-items:center;gap:6px}.secret-input>input{flex:1;min-width:0}.secret-input>button{flex-shrink:0;min-width:36px}.category-fields{border:0;padding:0;margin:0;min-width:0}.profile-inline-editor{min-width:0;font-size:13px;line-height:1.6}.profile-inline-editor .field>span,.profile-inline-editor .check-field{font-size:12px}.profile-inline-editor input:not([type=checkbox]),.profile-inline-editor select,.profile-inline-editor textarea{font-size:13px;min-height:36px}.profile-inline-editor .helper{font-size:12px;line-height:1.7}.profile-inline-editor .model-switch-list strong{font-size:12px}.profile-inline-editor .model-switch-list small{font-size:10px}.category-advanced{grid-column:1/-1;border-top:1px solid var(--border,#dce2eb);padding-top:14px;margin-top:8px}.category-advanced summary{cursor:pointer;font-size:13px;margin-bottom:12px}
.profile-section-actions{display:flex;align-items:center;justify-content:space-between;gap:16px;margin-top:20px;padding-top:16px;border-top:1px solid var(--border,#dce2eb)}
.profile-section-actions>span{font-size:12px;color:var(--muted,#64748b);line-height:1.6;max-width:620px}
.profile-section-actions>button{flex-shrink:0}
@media(max-width:760px){.profile-section-actions{align-items:stretch;flex-direction:column}}

.policy-source{padding:12px;margin:12px 0;font-size:12px;line-height:1.6}
.policy-source p{margin:0 0 8px}
.policy-preview{margin-top:12px}
.policy-preview pre{max-height:220px;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere;padding:10px;background:var(--app-bg);border-radius:6px}
.policy-preview button+button{margin-left:8px}
</style>
