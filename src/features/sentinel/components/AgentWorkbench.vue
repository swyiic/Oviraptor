<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { Check, FolderOpen, Plus, Save, Zap } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { GapFollowupPreview, ClosureHandoffPreview, SentinelScan } from "../types";
import type { Project } from "../../workspaces/types";
import { useClosureHandoff } from "../workbench/useClosureHandoff";
import { useBrowserAuthSessions } from "../workbench/useBrowserAuthSessions";
import { useSkillsCatalog } from "../../skills/useSkillsCatalog";
import { useFollowupRecovery } from "../workbench/useFollowupRecovery";
import { useWorkbenchTaskCreation, type WorkbenchMode } from "../workbench/useWorkbenchTaskCreation";
import WorkbenchGreyboxSettings from "../workbench/WorkbenchGreyboxSettings.vue";
import WorkbenchCicdSettings from "../workbench/WorkbenchCicdSettings.vue";
import WorkbenchOverview from "../workbench/WorkbenchOverview.vue";
import WorkbenchSkillSelection from "../workbench/WorkbenchSkillSelection.vue";
import WorkbenchBrowserAuth from "../workbench/WorkbenchBrowserAuth.vue";
import WorkbenchTaskNotices from "../workbench/WorkbenchTaskNotices.vue";
import { skillsApi } from "../../skills/api";
import { useWorkbenchPresentation } from "../workbench/useWorkbenchPresentation";

const props = defineProps<{
  projects: Project[];
  scans: SentinelScan[];
  projectId?: number;
  initialMode?: "web" | "code" | "greybox" | "cicd" | "skills";
  followup?: GapFollowupPreview;
  handoff?: ClosureHandoffPreview;
}>();
const emit = defineEmits<{
  notify: [type: "success" | "error" | "info", text: string];
  reload: [];
  openScan: [scan: SentinelScan];
  prepareScan: [scan: SentinelScan];
  createProject: [];
}>();
const { tr } = useI18n();
type Mode = WorkbenchMode;
const mode = ref<Mode>(props.initialMode || "web");
const scanModes: Array<{ key: Exclude<Mode, "skills">; label: [string, string]; detail: [string, string] }> = [
  { key: "web", label: ["Web 扫描", "Web scan"], detail: ["授权 URL 与业务接口", "Authorized URLs and business APIs"] },
  { key: "code", label: ["代码审计", "Code audit"], detail: ["仓库、SAST 与依赖", "Repository, SAST and dependencies"] },
  { key: "greybox", label: ["灰盒联测", "Greybox testing"], detail: ["运行环境 + 源码关联", "Live environment + source correlation"] },
  { key: "cicd", label: ["CI/CD", "CI/CD"], detail: ["流水线与发布门禁", "Pipeline and release gate"] },
];
const busy = ref(false);
const packageSkillIds = ref<number[]>([]);
const activeProjects = computed(() => props.projects.filter((project) => project.status !== "archived"));
const initialProjectId =
  activeProjects.value.find((project) => project.id === (props.handoff?.projectId || props.followup?.projectId || props.projectId))?.id ||
  activeProjects.value[0]?.id ||
  0;
const form = reactive({
  projectId: initialProjectId,
  taskName: props.handoff ? tr("独立关联任务", "Independent linked task") : props.followup ? tr("补充证据任务", "Evidence follow-up task") : "",
  urls: props.handoff?.targetUrls.join("\n") || props.followup?.targetUrl || "",
  sourcePath: "",
  skillIds: [] as number[],
  instruction: "",
  // Scan-strength picker was removed; default to the generous "deep" budget so
  // the executor isn't starved to ~1 model request by the standard cap. The
  // backend still enforces its hard token/request ceiling as the wallet.
  scanMode: "deep" as "quick" | "standard" | "deep",
  closure: "breadth" as "breadth" | "proof",
  orchestrationMode: "multi" as "single" | "multi",
  homogeneousWorkers: false,
  homogeneousExecution: false,
  approvedToolIds: [] as string[],
  allowSourceFetch: false,
  // Mandatory switch: high-risk capability classes, all off by default. A
  // non-empty selection requires legalAttestation before the task is created.
  highRiskClasses: [] as string[],
  legalAttestation: false,
  scopeMode: "full" as "auto" | "diff" | "full",
  diffBase: "origin/main",
  // Scan-strength / cost-budget pickers were removed from the form: the backend
  // uses its default budget cap. Kept on the form object (undefined) so task
  // creation keeps passing a value without a user-facing knob.
  maxBudgetUsd: undefined as number | undefined,
  environment: "staging",
  authProfileName: "",
  authType: "none" as "none" | "cookie" | "bearer" | "header",
  authHeaderName: "",
  authValue: "",
  authSessionId: "",
  authSessionIds: [] as string[],
  authLoginUrl: "",
  authSessionName: "",
  ciProvider: "github",
  repositoryUrl: "",
  branch: "main",
  commitSha: "",
  buildId: "",
  maxCritical: 0,
  maxHigh: 5,
  blockRelease: true,
});
const skillCatalog = useSkillsCatalog({
  tr, notify: (type, text) => emit("notify", type, text),
  clearSelection: () => { form.skillIds = []; },
});
const { skills } = skillCatalog;
let taskViewGeneration = 0;
let disposed = false;
const authControls = useBrowserAuthSessions({
  form, tr, isDisposed: () => disposed, notify: (type, text) => emit("notify", type, text),
});
const { authSessionScopeId, showAuthSessionPicker,
  resetIdentitySelection, loadAuthSessions } = authControls;
const followupControls = useFollowupRecovery({
  source: () => props.followup, generation: () => taskViewGeneration,
  isDisposed: () => disposed, form, busy, resetIdentitySelection,
  notify: (type, text) => emit("notify", type, text),
});
const { pendingFollowupInput, followupSubmission, restoreFollowupSubmission } = followupControls;
const handoffControls = useClosureHandoff({
  source: () => props.handoff,
  hasFollowup: () => Boolean(props.followup),
  isWebMode: () => mode.value === "web",
  generation: () => taskViewGeneration,
  isCurrent: (source, generation) => !disposed && generation === taskViewGeneration && props.handoff === source,
  form, busy, authSessionScopeId, tr,
  resetIdentitySelection: (scopeId) => {
    if (authSessionScopeId.value !== scopeId) return;
    resetIdentitySelection();
  },
  notify: (type, text) => emit("notify", type, text),
  reload: () => emit("reload"),
  prepareScan: (scan) => emit("prepareScan", scan),
  openScan: (scan) => emit("openScan", scan),
});
const { saveClosureHandoff } = handoffControls;
watch(() => [mode.value, form.projectId, props.followup?.sourceScanId,
  props.handoff?.sourceScanId, props.handoff?.closureId, props.handoff?.sourceHash,
  props.followup?.assessmentMessageId, props.followup?.sourceHash,
  props.followup?.projectId, props.followup?.targetUrl],
() => { taskViewGeneration++; }, { flush: "sync" });
const { start } = useWorkbenchTaskCreation({
  mode, form, busy, followup: () => props.followup,
  hasHandoff: () => Boolean(props.handoff), saveClosureHandoff,
  restoreFollowupSubmission, pendingFollowupInput, followupSubmission,
  authSessionScopeId, resetIdentitySelection,
  generation: () => taskViewGeneration, isDisposed: () => disposed,
  tr, notify: (type, text) => emit("notify", type, text),
  reload: () => emit("reload"), prepareScan: (scan) => emit("prepareScan", scan),
  openScan: (scan) => emit("openScan", scan),
});

// Canonical high-risk capability classes (mirror of the Rust CapabilityClass
// high-risk variants). Order here is the display order; the ids are what the
// backend authorization table stores.
const HIGH_RISK_CLASSES = ["code_exec", "target_write", "egress", "internal_scope", "persistence", "exfil", "waf_evasion"] as const;
function highRiskLabel(cls: string): string {
  switch (cls) {
    case "code_exec": return tr("命令执行 / RCE", "Command execution / RCE");
    case "target_write": return tr("对目标写入或改动", "Write / modify the target");
    case "egress": return tr("出网与 OOB 外带（DNSlog 等）", "Egress & OOB callback (DNSlog etc.)");
    case "internal_scope": return tr("内网 / SSRF 深入", "Internal / SSRF reach");
    case "persistence": return tr("在目标上建立持久化", "Establish persistence on target");
    case "exfil": return tr("数据外带", "Data exfiltration");
    case "waf_evasion": return tr("WAF 智能绕过", "Intelligent WAF evasion");
    default: return cls;
  }
}
function highRiskHint(cls: string): string {
  switch (cls) {
    case "code_exec": return tr("仅无害命令（id/whoami/ifconfig）可自动执行；其他命令逐次人工批准。", "Only harmless commands (id/whoami/ifconfig) auto-run; anything else needs per-action approval.");
    case "target_write": return tr("任何写接口 / 变更类请求都需人工批准。", "Any write / mutating request needs per-action approval.");
    case "egress": return tr("用于验证盲注类问题，可配置自备 DNSlog。", "For blind-class verification; a self-hosted DNSlog can be configured.");
    case "internal_scope": return tr("放行解析到内网/环回的目标（换协议深入）；SSRF 确定性守卫其余判定不变。", "Allows reaching internal/loopback targets (protocol pivots); the rest of the deterministic SSRF guard is unchanged.");
    case "persistence": return tr("高度不可逆，默认证明即止、可回滚。", "Highly irreversible; proof-only and reversible by default.");
    case "exfil": return tr("仅取证明影响所需的最小样本。", "Only the minimal sample needed to prove impact.");
    case "waf_evasion": return tr("允许对拦截进行多次智能绕过尝试。", "Allows multiple intelligent evasion attempts against blocking.");
    default: return "";
  }
}

watch(
  () => props.initialMode,
  (value) => {
    if (value) mode.value = value;
  },
);
watch(
  () => form.projectId,
  async (value) => {
    resetIdentitySelection();
    if (value) await loadAuthSessions();
  },
);
watch(
  () => props.projectId,
  (value) => {
    if (value && activeProjects.value.some((project) => project.id === value))
      form.projectId = value;
    else if (value) form.projectId = 0;
  },
);
watch(
  () => props.projects,
  () => {
    if (!activeProjects.value.some((project) => project.id === form.projectId))
      form.projectId =
        activeProjects.value.find((project) => project.id === props.projectId)?.id ||
        activeProjects.value[0]?.id ||
        0;
  },
  { deep: true },
);
watch(mode, (value) => {
  form.scanMode = value === "cicd" ? "quick" : "deep";
  form.scopeMode = value === "cicd" ? "auto" : "full";
  form.skillIds = [];
  showAuthSessionPicker.value = false;
});
const { selectedSkills, modeLabel, chooseSource } = useWorkbenchPresentation({ mode, form, skills, tr });
onMounted(async () => {
  try {
    const packages = await skillsApi.listPackages();
    // Hide only BUILT-IN package skills (e.g. the Compound Engineering ce-* set) from
    // the per-task picker. The operator's own imported packages (e.g. secknowledge-skill,
    // src-bounty-skill) must stay selectable per task, including on Web.
    packageSkillIds.value = packages
      .filter((pack) => pack.builtin)
      .flatMap((pack) => pack.skills.map((skill) => skill.id));
  } catch {
    // Picker falls back to showing all skills if packages cannot be read.
  }
});
onBeforeUnmount(() => {
  disposed = true;
});
</script>

<template>
  <section class="agent-workbench">
    <nav v-if="mode !== 'skills'" class="agent-mode-switch" aria-label="Nest scan type">
      <button v-for="item in scanModes" :key="item.key" type="button" :disabled="Boolean(followup || handoff)" :class="{ active: mode === item.key }" @click="mode = item.key">
        <strong>{{ tr(...item.label) }}</strong><small>{{ tr(...item.detail) }}</small>
      </button>
    </nav>
    <template v-if="mode !== 'skills'">
      <div class="workbench-grid">
        <section class="panel workbench-form">
          <div class="panel-heading">
            <div>
              <h3>{{ modeLabel(mode) }}</h3>
              <p v-if="mode === 'web' && form.homogeneousWorkers && !form.homogeneousExecution">{{ tr("主管按观测范围动态拆分任务，按需唤起同质 Worker 分析已有证据并汇报；本阶段只读分析，不派发目标验证或确认漏洞。", "The coordinator decomposes observed scope and wakes identical Workers to analyze existing evidence. This phase is read-only; it does not dispatch target verification or confirm findings.") }}</p>
              <p v-else-if="mode === 'web' && form.homogeneousWorkers">{{ tr("主管按目标分派任务，冻结批准工具和权限；解析辅助代码在无网络沙箱中执行，实际结果与覆盖缺口进入聊天。", "The coordinator assigns work by goal with frozen approved tools and permissions. Parsing helpers execute in an offline sandbox; chat shows actual results and coverage gaps.") }}</p>
              <p v-else-if="mode === 'web'">
                {{
                  tr(
                    "输入 URL 后自动完成页面探索、HTTP/参数捕获、JS/指纹、本地知识与确定性验证；标准扫描处理真实运行时 API，深度扫描优先验证高价值证据。",
                    "Explore the page, capture HTTP and parameters, analyze JS/fingerprints, and match local knowledge; Standard scans cover observed runtime APIs while Deep scans prioritize high-value evidence."
                  )
                }}
              </p>
              <p v-else-if="mode === 'code'">
                {{
                  tr(
                    "完整阅读本地仓库，验证可复现的代码安全问题。",
                    "Review a local repository and verify reproducible security issues."
                  )
                }}
              </p>
              <p v-else-if="mode === 'greybox'">
                {{
                  tr(
                    "把运行中的 URL 与本地源码同时交给原生 Agent，关联请求与实现。",
                    "Give the native Agent both live URLs and local source to connect requests with implementation."
                  )
                }}
              </p>
              <p v-else>
                {{
                  tr(
                    "以快速或变更范围模式审计当前分支，适合提交前检查。",
                    "Audit the current branch in quick or diff scope for pre-commit checks."
                  )
                }}
              </p>
            </div>
          </div>
          <section v-if="!activeProjects.length" class="workspace-empty-callout">
            <Plus :size="22" />
            <div><strong>{{ tr("先建立一个工作空间", "Create a workspace first") }}</strong><p>{{ tr("资产、登录会话、扫描、证据和知识都会绑定到同一工作空间，不会再出现孤立任务。", "Assets, login sessions, scans, evidence, and knowledge stay in one workspace.") }}</p></div>
            <button class="button primary" type="button" @click="emit('createProject')">{{ tr("立即新建工作空间", "Create workspace") }}</button>
          </section>
          <WorkbenchTaskNotices :handoff="handoff" :followup="followup" :busy="busy"
            :handoff-controls="handoffControls" :followup-controls="followupControls" />
          <div class="workbench-fields">
            <div class="project-field-with-action">
              <label class="field"
                ><span>{{ tr("归属工作空间", "Workspace") }}</span
                ><select v-model.number="form.projectId" :disabled="Boolean(followup || handoff)">
                  <option :value="0">{{ tr("请选择或新建", "Select or create") }}</option>
                  <option v-for="project in activeProjects" :key="project.id" :value="project.id">{{ project.name }}</option>
                </select></label
              >
              <button class="button ghost" type="button" @click="emit('createProject')"><Plus :size="14" />{{ tr("新建空间", "New workspace") }}</button>
            </div>
            <label class="field"
              ><span>{{ tr("任务名称", "Task name") }}</span
              ><input
                v-model="form.taskName"
                :placeholder="`${modeLabel(mode)} · ${new Date().toLocaleDateString()}`"
            /></label>
            <label v-if="mode !== 'web'" class="field span-two"
              ><span>{{ tr("源码目录", "Source directory") }}</span>
              <div class="path-picker">
                <input
                  v-model="form.sourcePath"
                  readonly
                  :placeholder="
                    tr('选择本地仓库目录', 'Choose a local repository')
                  "
                /><button class="button ghost" @click="chooseSource">
                  <FolderOpen :size="15" />{{ tr("选择", "Choose") }}
                </button>
              </div></label
            >
            <label v-if="mode === 'web'" class="field span-two"
              ><span>{{ tr("授权 Web URL（每行一个）", "Authorized Web URLs (one per line)") }}</span
              ><textarea
                v-model="form.urls"
                :readonly="Boolean(followup)"
                rows="8"
                placeholder="https://app.example.com\nhttps://admin.example.com"
              ></textarea>
              <small class="helper">{{ tr("能打开的页面会直接进入调查。尽量多找会留下「已测未发现」；只证明风险时，没有控制请求、测试请求和影响说明的结论不进漏洞列表。", "Reachable pages are investigated. Breadth keeps tested-without-finding notes. Proof only lists findings that bind a control request, a test request, and an impact statement.") }}</small>
            </label>
            <label v-if="mode === 'web'" class="field span-two source-fetch-optin">
              <span class="checkbox-row"><input type="checkbox" v-model="form.allowSourceFetch" />{{ tr("允许拉取开源上游源码进行审计复现", "Allow fetching the open-source upstream for source-audit reproduction") }}</span>
              <small class="helper">{{ tr("仅在勾选时，识别到的开源系统才会从受控白名单（github/gitee/gitlab）拉取上游源码到隔离区，供灰盒复现；不勾选绝不联网拉源。上游仅作近似参照，魔改部署以灰盒实测为准。", "Only when checked will an identified OSS product's upstream be fetched from the controlled allowlist (github/gitee/gitlab) into quarantine for greybox reproduction. Never fetched otherwise. Upstream is an approximate oracle; a modified deployment is judged by the greybox run.") }}</small>
            </label>
            <div v-if="mode === 'web'" class="field span-two high-risk-optin">
              <span class="high-risk-title">{{ tr("高危能力授权（默认全部关闭）", "High-risk capability authorization (all off by default)") }}</span>
              <small class="helper">{{ tr("只有在这里勾选的能力类才可能被使用；即便勾选，每个真实高危动作在运行时仍需人工逐次批准（无害命令 id/whoami/ifconfig 除外）。未授权时整条执行链拒绝这些操作。", "Only classes checked here may ever be used; even then every real high-risk action still needs per-action human approval at run time (harmless commands id/whoami/ifconfig excepted). Without authorization the whole execution chain refuses these operations.") }}</small>
              <div class="high-risk-grid">
                <label v-for="cls in HIGH_RISK_CLASSES" :key="cls" class="checkbox-row high-risk-item">
                  <input type="checkbox" :value="cls" v-model="form.highRiskClasses" />
                  <span class="high-risk-item-text"><strong>{{ highRiskLabel(cls) }}</strong><em>{{ highRiskHint(cls) }}</em></span>
                </label>
              </div>
              <label class="checkbox-row high-risk-attest" :class="{ 'attest-missing': form.highRiskClasses.length && !form.legalAttestation }">
                <input type="checkbox" v-model="form.legalAttestation" :disabled="!form.highRiskClasses.length" />
                {{ tr("我确认已获得对以上目标的合法书面授权，并对使用这些能力负责。", "I confirm I hold written legal authorization for these targets and take responsibility for using these capabilities.") }}
              </label>
              <small v-if="form.highRiskClasses.length && !form.legalAttestation" class="helper high-risk-warn">{{ tr("已选择高危能力：必须勾选合法授权确认后才能创建任务。", "High-risk capabilities selected: you must confirm legal authorization before the task can be created.") }}</small>
            </div>
            <label v-if="mode === 'greybox'" class="field span-two"
              ><span>{{
                tr("授权测试 URL（每行一个）", "Authorized URLs (one per line)")
              }}</span
              ><textarea
                v-model="form.urls"
                rows="4"
                placeholder="https://staging.example.com"
              ></textarea>
            </label>
            <WorkbenchBrowserAuth v-if="mode === 'web' || mode === 'greybox'" :form="form" :controls="authControls" />
            <WorkbenchGreyboxSettings v-if="mode === 'greybox'" :form="form" />
            <WorkbenchCicdSettings v-if="mode === 'cicd'" :form="form" />
            <label v-if="mode !== 'web'" class="field"
              ><span>{{ tr("代码范围", "Code scope") }}</span
              ><select v-model="form.scopeMode">
                <option value="auto">{{ tr("Auto：增量优先，失败时整仓", "Auto: diff first, full fallback") }}</option>
                <option value="diff">{{ tr("Diff：仅变更文件，不扩大范围", "Diff: changed files only, no widening") }}</option>
                <option value="full">{{ tr("Full：完整源码范围", "Full: complete source scope") }}</option>
              </select></label
            >
            <p v-if="mode !== 'web'" class="form-hint span-two" role="note">
              {{ form.scopeMode === 'auto'
                ? tr("Auto 允许在对比基线或变更清单不可用时转为整仓分析，并在结果中记录原因。选择增量时沿用 Diff 的 CodeQL 项目上下文覆盖限制。", "Auto permits full analysis when the base or change manifest is unavailable; the reason is recorded. When diff is selected, the same CodeQL project-context coverage limit applies.")
                : form.scopeMode === 'diff'
                  ? tr("Diff 基线无效或变更清单不可用时不执行整仓分析。空增量不代表安全；CodeQL 项目上下文尚未支持增量授权，将记录覆盖缺口。", "Diff never falls back to full analysis. An empty diff is not a clean result; CodeQL project context is not yet supported for diff scope and is reported as a gap.")
                  : tr("Full 使用完整源码快照，不使用隐藏的旧对比基线。依赖缓存、构建输出等排除项仍会记录。", "Full uses the complete source snapshot and ignores stale hidden bases. Exclusions such as dependency caches and build output remain recorded.") }}
            </p>
            <label
              v-if="mode !== 'web' && form.scopeMode !== 'full'"
              class="field"
              ><span>{{ tr("对比分支/提交", "Diff base") }}</span
              ><input v-model="form.diffBase" placeholder="origin/main"
            /></label>
            <WorkbenchSkillSelection :mode="mode" :expanded="true" :skills="skills" :form="form" :hidden-skill-ids="packageSkillIds" />
          </div>
          <div class="selected-skills">
            <span v-for="skill in selectedSkills" :key="skill.id"
              ><Check :size="12" />{{ skill.name }}</span
            >
          </div>
          <footer>
            <small>{{
              tr(
                "第三方库默认只做清单与已知版本风险检查；业务 JS 和应用分包才会深度解析。",
                "Third-party libraries are inventoried and version-checked; business JS and app chunks receive deep analysis."
              )
            }}</small
            ><button v-if="mode === 'web'" class="button ghost" :disabled="busy" @click="start(true)">
              <Save :size="15" />{{ tr("保存草稿 / 配置控制组", "Save draft / configure controls") }}
            </button><button v-if="!followup && !handoff" class="button primary" :disabled="busy" @click="start()">
              <Zap :size="15" />{{
                busy
                  ? tr("启动中…", "Starting…")
                  : mode === 'web'
                    ? tr("启动扫描", "Start scan")
                    : tr("启动 Nest", "Start Nest")
              }}
            </button>
          </footer>
        </section>
        <WorkbenchOverview :scans="scans" :mode="mode" :mode-label="modeLabel" @open-scan="emit('openScan', $event)" />
      </div>
    </template>
  </section>
</template>

<style scoped src="./workbenchPresentation.css"></style>
