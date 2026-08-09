<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import {
  BookOpen,
  Braces,
  Check,
  Download,
  FolderOpen,
  GitBranch,
  KeyRound,
  LogIn,
  Plus,
  Save,
  ShieldCheck,
  RefreshCw,
  Sparkles,
  Trash2,
  Upload,
  X,
  Zap,
} from "@lucide/vue";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import { useI18n } from "../i18n";
import type {
  BrowserAuthSession,
  Project,
  SentinelScan,
  StrixSkill,
} from "../types";
import InlineConfirm from "./InlineConfirm.vue";

const props = defineProps<{
  projects: Project[];
  scans: SentinelScan[];
  projectId?: number;
  initialMode?: "web" | "code" | "greybox" | "cicd" | "skills";
}>();
const emit = defineEmits<{
  notify: [type: "success" | "error" | "info", text: string];
  reload: [];
  openScan: [scan: SentinelScan];
  createProject: [];
}>();
const { tr } = useI18n();
type Mode = "web" | "code" | "greybox" | "cicd" | "skills";
const mode = ref<Mode>(props.initialMode || "code");
const scanModes: Array<{ key: Exclude<Mode, "skills">; label: string; detail: string }> = [
  { key: "web", label: "Web 扫描", detail: "授权 URL 与业务接口" },
  { key: "code", label: "代码审计", detail: "仓库、SAST 与依赖" },
  { key: "greybox", label: "灰盒联测", detail: "运行环境 + 源码关联" },
  { key: "cicd", label: "CI/CD", detail: "流水线与发布门禁" },
];
const skills = ref<StrixSkill[]>([]);
const busy = ref(false);
const authBusy = ref("");
const authSessions = ref<BrowserAuthSession[]>([]);
const showAdvanced = ref(false);
const webPreset = ref<"bounded" | "balanced" | "evidence">("balanced");
const skillBusy = ref(false);
const refiningSkillId = ref<number>();
const deleteSkill = ref<StrixSkill>();
const editingSkill = ref<StrixSkill>();
const showSkillEditor = ref(false);
const skillForm = reactive({
  name: "",
  description: "",
  instructions: "",
  enabled: true,
});
const activeProjects = computed(() => props.projects.filter((project) => project.status !== "archived"));
const initialProjectId =
  activeProjects.value.find((project) => project.id === props.projectId)?.id ||
  activeProjects.value[0]?.id ||
  0;
const form = reactive({
  projectId: initialProjectId,
  taskName: "",
  urls: "",
  sourcePath: "",
  skillIds: [] as number[],
  instruction: "",
  scanMode: "standard" as "quick" | "standard" | "deep",
  scopeMode: "full" as "auto" | "diff" | "full",
  diffBase: "origin/main",
  maxBudgetUsd: 1.5 as number | undefined,
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

watch(
  () => props.initialMode,
  (value) => {
    if (value) mode.value = value;
  },
);
watch(
  () => form.projectId,
  async (value) => {
    form.authSessionId = "";
    form.authSessionIds = [];
    if (value) await loadAuthSessions();
    else authSessions.value = [];
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
  form.scanMode = value === "cicd" ? "quick" : value === "web" ? "standard" : "deep";
  form.scopeMode = value === "cicd" ? "auto" : "full";
  form.skillIds = [];
  showAdvanced.value = false;
});
function applyWebPreset(value: "bounded" | "balanced" | "evidence") {
  webPreset.value = value;
  if (value === "bounded") {
    form.scanMode = "quick";
    form.maxBudgetUsd = 0.5;
  } else if (value === "balanced") {
    form.scanMode = "standard";
    form.maxBudgetUsd = 1.5;
  } else {
    form.scanMode = "deep";
    form.maxBudgetUsd = 4;
  }
}
const modeScans = computed(() =>
  props.scans.filter((scan) => scan.scanType === mode.value),
);
const tokenByMode = computed(() =>
  ["web", "code", "greybox", "cicd"].map((key) => {
    const rows = props.scans.filter((scan) => scan.scanType === key);
    return {
      key,
      input: rows.reduce((sum, scan) => sum + scan.inputTokens, 0),
      output: rows.reduce((sum, scan) => sum + scan.outputTokens, 0),
      total: rows.reduce(
        (sum, scan) =>
          sum + (scan.totalTokens || scan.inputTokens + scan.outputTokens),
        0,
      ),
    };
  }),
);
const selectedSkills = computed(() =>
  skills.value.filter((skill) => form.skillIds.includes(skill.id)),
);
const skillPreview = computed(() => {
  const source = skillForm.instructions.trim();
  if (!source) return [] as Array<{ heading: string; body: string }>;
  const sections: Array<{ heading: string; body: string }> = [];
  let current = { heading: tr("未命名章节", "Untitled section"), body: "" };
  for (const line of source.split(/\r?\n/)) {
    const heading = line.match(/^#{1,3}\s+(.+)$/);
    if (heading) {
      if (current.body.trim()) sections.push({ ...current, body: current.body.trim() });
      current = { heading: heading[1].trim(), body: "" };
    } else {
      current.body += `${line}\n`;
    }
  }
  if (current.body.trim()) sections.push({ ...current, body: current.body.trim() });
  return sections;
});
function format(value: number) {
  return new Intl.NumberFormat("zh-CN", {
    notation: value > 999999 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value || 0);
}
function modeLabel(value: string) {
  return (
    (
      {
        web: tr("Strix Web 扫描", "Strix Web scan"),
        code: tr("代码审计", "Code audit"),
        greybox: tr("灰盒联测", "Grey-box"),
        cicd: "CI/CD",
      } as Record<string, string>
    )[value] || value
  );
}
function deploymentLabel(scan: SentinelScan) {
  if (scan.llmDeployment === "local") {
    return scan.llmFullPower
      ? tr("本地 LLM · 火力全开", "Local LLM · full power")
      : tr("本地 LLM", "Local LLM");
  }
  if (scan.llmDeployment === "cloud") return tr("云端 AI", "Cloud AI");
  return tr("LLM 未记录", "LLM not recorded");
}
async function loadSkills() {
  try {
    skills.value = await api.listStrixSkills();
    form.skillIds = [];
  } catch (error) {
    emit("notify", "error", String(error));
  }
}
async function loadAuthSessions() {
  if (!form.projectId) {
    authSessions.value = [];
    return;
  }
  try {
    authSessions.value = await api.listBrowserAuthSessions(form.projectId);
    const validIds = new Set(authSessions.value.filter((session) => session.status === "valid").map((session) => session.id));
    form.authSessionIds = form.authSessionIds.filter((id) => validIds.has(id));
    if (!form.authSessionIds.length) {
      const first = authSessions.value.find((session) => session.status === "valid")?.id;
      if (first) form.authSessionIds = [first];
    }
    form.authSessionId = form.authSessionIds[0] || "";
  } catch (error) {
    emit("notify", "error", String(error));
  }
}
function firstUrl() {
  return form.urls
    .split(/\r?\n|,/)
    .map((value) => value.trim())
    .find(Boolean) || "";
}
function authStatusLabel(status: string) {
  return ({
    valid: tr("会话有效", "Session active"),
    capturing: tr("等待完成登录", "Waiting for login"),
    needs_check: tr("需要确认", "Needs check"),
    invalid: tr("会话失效", "Session invalid"),
    expired: tr("会话过期", "Session expired"),
  } as Record<string, string>)[status] || status;
}
function shortTime(value: string) {
  if (!value) return tr("尚未校验", "Not validated");
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}
async function openLogin(session?: BrowserAuthSession) {
  if (!form.projectId) {
    emit("notify", "info", tr("请先选择工作空间", "Select a workspace first"));
    return;
  }
  const entryUrl = (form.authLoginUrl || session?.entryUrl || firstUrl() || "").trim();
  if (!/^https?:\/\//i.test(entryUrl)) {
    emit("notify", "info", tr("请先填写登录 URL 或授权 URL", "Enter a login or authorized URL first"));
    return;
  }
  authBusy.value = session?.id || "new";
  try {
    const opened = await api.openBrowserAuthSession({
      id: session?.id,
      projectId: form.projectId,
      name: form.authSessionName || session?.name || "",
      entryUrl,
    });
    if (!form.authSessionIds.includes(opened.id)) form.authSessionIds.push(opened.id);
    form.authSessionId = form.authSessionIds[0] || opened.id;
    await loadAuthSessions();
    emit("notify", "info", tr("登录窗口已打开：处理验证码并登录，进入任意后台功能后回到这里完成捕获", "Login window opened. Sign in, visit one authenticated feature, then return here to finish capture."));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authBusy.value = "";
  }
}
async function finishLogin(session: BrowserAuthSession) {
  authBusy.value = session.id;
  try {
    const updated = await api.finishBrowserAuthSession(session.id);
    if (!form.authSessionIds.includes(updated.id)) form.authSessionIds.push(updated.id);
    form.authSessionId = form.authSessionIds[0] || updated.id;
    await loadAuthSessions();
    emit("notify", updated.status === "valid" ? "success" : "info", updated.status === "valid"
      ? tr("登录会话已捕获，绿灯亮起；后续探测会自动复用", "Session captured and active; later probes will reuse it")
      : updated.lastError || tr("已捕获，但需要再访问一个登录后功能", "Captured, but visit an authenticated feature first"));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authBusy.value = "";
  }
}
async function validateLogin(session: BrowserAuthSession) {
  authBusy.value = session.id;
  try {
    const updated = await api.validateBrowserAuthSession(session.id);
    await loadAuthSessions();
    emit("notify", updated.status === "valid" ? "success" : "info", updated.status === "valid"
      ? tr("会话校验通过", "Session validation passed")
      : updated.lastError || tr("会话需要重新确认", "Session needs confirmation"));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authBusy.value = "";
  }
}
async function removeLogin(session: BrowserAuthSession) {
  if (!window.confirm(tr(`删除登录会话“${session.name}”？之后需要重新登录。`, `Delete session “${session.name}”? You will need to sign in again.`))) return;
  authBusy.value = session.id;
  try {
    await api.deleteBrowserAuthSession(session.id);
    form.authSessionIds = form.authSessionIds.filter((id) => id !== session.id);
    form.authSessionId = form.authSessionIds[0] || "";
    await loadAuthSessions();
    emit("notify", "success", tr("登录会话已删除", "Login session deleted"));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authBusy.value = "";
  }
}
async function refineSkill(skill: StrixSkill) {
  refiningSkillId.value = skill.id;
  try {
    const result = await api.refineStrixSkillWithKnowledge(skill.id);
    await loadSkills();
    emit(
      "notify",
      "success",
      result === skill.id
        ? tr("已用最新高质量知识精炼 Skill", "Skill refined with the latest high-quality knowledge")
        : tr("内置 Skill 未被覆盖，已创建增强副本", "Built-in Skill kept intact; an enhanced copy was created"),
    );
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    refiningSkillId.value = undefined;
  }
}
async function exportSkills() {
  try {
    const path = await api.exportStrixSkills();
    emit("notify", "success", tr(`Skills 已导出：${path}`, `Skills exported: ${path}`));
  } catch (error) {
    emit("notify", "error", String(error));
  }
}
async function importSkills() {
  const path = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Oviraptor Skills", extensions: ["json"] }],
  });
  if (!path) return;
  try {
    const count = await api.importStrixSkills(path);
    await loadSkills();
    emit("notify", "success", tr(`已导入 ${count} 个 Skill`, `Imported ${count} Skills`));
  } catch (error) {
    emit("notify", "error", String(error));
  }
}
async function importInternalSecSkills() {
  const path = await open({
    multiple: false,
    directory: true,
    title: tr("选择内部 sec_skills 目录", "Select internal sec_skills directory"),
  });
  if (!path) return;
  try {
    const result = await api.importSecSkillKnowledge(path);
    await loadSkills();
    emit(
      "notify",
      "success",
      tr(
        `已完整导入 ${String(result.filesScanned || 0)} 个内部 Skill 文件`,
        `Imported ${String(result.filesScanned || 0)} internal Skill files`,
      ),
    );
  } catch (error) {
    emit("notify", "error", String(error));
  }
}
async function chooseSource() {
  const value = await open({
    directory: true,
    multiple: false,
    title: tr(
      "选择源码目录（只读挂载）",
      "Select source directory (read-only mount)",
    ),
  });
  if (value) form.sourcePath = value;
}
async function start() {
  if (!form.projectId) {
    emit("notify", "info", tr("请先选择归属工作空间", "Select a workspace"));
    return;
  }
  busy.value = true;
  try {
    const urls = form.urls
      .split(/\r?\n|,/)
      .map((value) => value.trim())
      .filter(Boolean);
    if (mode.value === "web") {
      if (!urls.length) {
        throw new Error(
          tr(
            "至少添加一个 http:// 或 https:// URL",
            "Add at least one http:// or https:// URL",
          ),
        );
      }
      const draft = await api.createSentinelUrlScan(
        form.projectId,
        form.taskName,
        urls,
        form.scanMode,
        form.maxBudgetUsd,
        form.authSessionId || undefined,
        form.authSessionIds,
      );
      const scan = await api.confirmSentinelScan(draft.id);
      emit("notify", "success", tr("Strix Web 扫描已启动", "Strix Web scan started"));
      emit("reload");
      form.taskName = "";
      form.urls = "";
      emit("openScan", scan);
      return;
    }
    const scan = await api.startStrixWorkbenchScan({
      projectId: form.projectId,
      taskName: form.taskName,
      scanType: mode.value as Exclude<Mode, "skills" | "web">,
      urls,
      sourcePath: form.sourcePath,
      skillIds: form.skillIds,
      instruction: form.instruction,
      scanMode: form.scanMode,
      scopeMode: form.scopeMode,
      diffBase: form.diffBase,
      maxBudgetUsd: form.maxBudgetUsd,
      environment: form.environment,
      authProfileName: form.authProfileName,
      authType: form.authType,
      authHeaderName: form.authHeaderName,
      authValue: form.authValue,
      authSessionId: form.authSessionId,
      authSessionIds: form.authSessionIds,
      ciProvider: form.ciProvider,
      repositoryUrl: form.repositoryUrl,
      branch: form.branch,
      commitSha: form.commitSha,
      buildId: form.buildId,
      maxCritical: form.maxCritical,
      maxHigh: form.maxHigh,
      blockRelease: form.blockRelease,
    });
    emit(
      "notify",
      "success",
      tr(
        "Strix 任务已启动，可在任务总览查看进度",
        "Strix task started; track it in Tasks",
      ),
    );
    emit("reload");
    form.taskName = "";
    form.instruction = "";
    form.authValue = "";
    emit("openScan", scan);
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    busy.value = false;
  }
}
function editSkill(skill?: StrixSkill) {
  editingSkill.value = skill;
  skillForm.name = skill?.name || "";
  skillForm.description = skill?.description || "";
  skillForm.instructions = skill?.instructions || "";
  skillForm.enabled = skill?.enabled ?? true;
  showSkillEditor.value = true;
}
function cloneBuiltinSkill(skill: StrixSkill) {
  editingSkill.value = undefined;
  skillForm.name = `${skill.name} · 自定义增强版`;
  skillForm.description = skill.description;
  skillForm.instructions = skill.instructions;
  skillForm.enabled = true;
  showSkillEditor.value = true;
}
function insertSkillTemplate() {
  skillForm.instructions = `## Objective\nDescribe the security outcome this skill should achieve.\n\n## Scope\n- Include: files, frameworks, routes, or vulnerability classes to inspect\n- Exclude: generated files, third-party bundles, destructive actions\n\n## Analysis workflow\n1. Establish evidence and affected target.\n2. Trace data flow or request flow.\n3. Verify impact safely and preserve Strix native PoC logic.\n\n## Output requirements\n- Bind every finding to a URL or source location.\n- Include evidence, impact, CVSS/CWE, remediation, and confidence.\n- Do not report reconnaissance-only observations as vulnerabilities.`;
}
async function saveSkill() {
  skillBusy.value = true;
  try {
    await api.saveStrixSkill({
      id: editingSkill.value?.id,
      name: skillForm.name,
      description: skillForm.description,
      instructions: skillForm.instructions,
      enabled: skillForm.enabled,
    });
    showSkillEditor.value = false;
    await loadSkills();
    emit(
      "notify",
      "success",
      tr("技能已保存，之后启动的任务会读取它", "Skill saved for future scans"),
    );
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    skillBusy.value = false;
  }
}
async function removeSkill() {
  if (!deleteSkill.value) return;
  skillBusy.value = true;
  try {
    await api.deleteStrixSkill(deleteSkill.value.id);
    deleteSkill.value = undefined;
    await loadSkills();
    emit("notify", "success", tr("自定义技能已删除", "Custom skill deleted"));
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    skillBusy.value = false;
  }
}
onMounted(async () => {
  await Promise.all([loadSkills(), loadAuthSessions()]);
});
</script>

<template>
  <section class="strix-workbench">
    <nav v-if="mode !== 'skills'" class="strix-mode-switch" aria-label="Strix scan type">
      <button v-for="item in scanModes" :key="item.key" type="button" :class="{ active: mode === item.key }" @click="mode = item.key">
        <strong>{{ tr(item.label, item.label) }}</strong><small>{{ tr(item.detail, item.detail) }}</small>
      </button>
    </nav>
    <template v-if="mode !== 'skills'">
      <div class="workbench-grid">
        <section class="panel workbench-form">
          <div class="panel-heading">
            <div>
              <span class="eyebrow">{{ mode.toUpperCase() }}</span>
              <h3>{{ modeLabel(mode) }}</h3>
              <p v-if="mode === 'web'">
                {{
                  tr(
                    "输入 URL 后自动完成页面探索、HTTP/参数捕获、JS/指纹、本地知识与确定性验证；只有高价值候选才进入 Strix。",
                    "Explore the page, capture HTTP and parameters, analyze JS/fingerprints, match local knowledge, and invoke Strix only for valuable candidates.",
                  )
                }}
              </p>
              <p v-else-if="mode === 'code'">
                {{
                  tr(
                    "完整阅读本地仓库，验证可复现的代码安全问题。",
                    "Review a local repository and verify reproducible security issues.",
                  )
                }}
              </p>
              <p v-else-if="mode === 'greybox'">
                {{
                  tr(
                    "把运行中的 URL 与本地源码同时交给 Strix，关联请求与实现。",
                    "Give Strix both live URLs and local source to connect requests with implementation.",
                  )
                }}
              </p>
              <p v-else>
                {{
                  tr(
                    "以快速或变更范围模式审计当前分支，适合提交前检查。",
                    "Audit the current branch in quick or diff scope for pre-commit checks.",
                  )
                }}
              </p>
            </div>
          </div>
          <section v-if="mode === 'web'" class="web-investigation-presets">
            <button :class="{ active: webPreset === 'bounded' }" @click="applyWebPreset('bounded')">
              <span>01</span><strong>快速保底</strong><small>确定性侦察优先，0.5 USD 上限</small>
            </button>
            <button :class="{ active: webPreset === 'balanced' }" @click="applyWebPreset('balanced')">
              <span>02</span><strong>标准自动调查</strong><small>推荐：完整流程，1.5 USD 上限</small>
            </button>
            <button :class="{ active: webPreset === 'evidence' }" @click="applyWebPreset('evidence')">
              <span>03</span><strong>证据后深挖</strong><small>已有高价值线索时使用，4 USD 上限</small>
            </button>
          </section>
          <section v-if="!activeProjects.length" class="workspace-empty-callout">
            <Plus :size="22" />
            <div><strong>{{ tr("先建立一个工作空间", "Create a workspace first") }}</strong><p>{{ tr("资产、登录会话、扫描、证据和知识都会绑定到同一工作空间，不会再出现孤立任务。", "Assets, login sessions, scans, evidence, and knowledge stay in one workspace.") }}</p></div>
            <button class="button primary" type="button" @click="emit('createProject')">{{ tr("立即新建工作空间", "Create workspace") }}</button>
          </section>
          <div class="workbench-fields">
            <div class="project-field-with-action">
              <label class="field"
                ><span>{{ tr("归属工作空间", "Workspace") }}</span
                ><select v-model.number="form.projectId">
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
                rows="8"
                placeholder="https://app.example.com\nhttps://admin.example.com"
              ></textarea>
              <small class="helper">{{ tr("提交后立即进入 Web 自适应扫描；低价值目标仍只保留前端结果。", "The task starts with Web adaptive scanning; low-value targets still keep frontend-only results.") }}</small>
            </label>
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
            <section v-if="mode === 'web' || mode === 'greybox'" class="browser-auth-center span-two">
              <header>
                <div class="browser-auth-title"><LogIn :size="17" /><div><strong>{{ tr("浏览器登录会话", "Browser login session") }}</strong><small>{{ tr("你只负责登录和动态验证码；Oviraptor 捕获 Cookie、Storage、认证头及实际请求头，后续探测自动复用。", "You only sign in and solve CAPTCHA; Oviraptor captures cookies, storage, auth headers, and effective request headers for later probes.") }}</small></div></div>
                <span class="auth-global-state" :class="{ valid: authSessions.some((session) => session.status === 'valid') }"><i></i>{{ authSessions.some((session) => session.status === 'valid') ? tr("已有有效会话", "Active session") : tr("尚未建立会话", "No active session") }}</span>
              </header>
              <div class="auth-session-create">
                <label class="field"><span>{{ tr("登录入口", "Login URL") }}</span><input v-model="form.authLoginUrl" :placeholder="firstUrl() || 'https://app.example.com/login'" /></label>
                <label class="field"><span>{{ tr("会话名称（可选）", "Session name (optional)") }}</span><input v-model="form.authSessionName" :placeholder="tr('测试管理员 / 普通用户', 'Test admin / normal user')" /></label>
                <button class="button primary auth-login-button" type="button" :disabled="Boolean(authBusy) || !form.projectId" @click="openLogin()"><LogIn :size="15" />{{ authBusy === 'new' ? tr("正在打开…", "Opening…") : tr("打开登录窗口", "Open login window") }}</button>
              </div>
              <div v-if="authSessions.length" class="auth-session-list">
                <p class="auth-identity-hint"><ShieldCheck :size="13" />{{ tr(`已选择 ${form.authSessionIds.length} 个身份；选择两个以上时会复用同一动作计划生成权限差异矩阵。`, `${form.authSessionIds.length} identities selected. Two or more produce a permission-difference matrix with the same action plan.`) }}</p>
                <article v-for="session in authSessions" :key="session.id" class="auth-session-card" :class="[{ selected: form.authSessionIds.includes(session.id) }, `status-${session.status}`]">
                  <label class="auth-session-select"><input v-model="form.authSessionIds" type="checkbox" :value="session.id" :disabled="session.status !== 'valid'" @change="form.authSessionId = form.authSessionIds[0] || ''" /><span class="auth-status-dot"></span><span><strong>{{ session.name }}</strong><small>{{ authStatusLabel(session.status) }} · {{ shortTime(session.lastValidatedAt || session.updatedAt) }}</small></span></label>
                  <div class="auth-session-metrics"><span><b>{{ session.cookieCount }}</b> Cookie</span><span><b>{{ session.headerCount }}</b> {{ tr("认证头", "auth headers") }}</span><span><b>{{ session.storageCount }}</b> Storage</span><span><b>{{ session.capturedRequestCount }}</b> {{ tr("请求", "requests") }}</span></div>
                  <p class="auth-session-scope"><span>{{ tr("作用域", "Scope") }}</span>{{ session.scopeHosts.slice(0, 5).join(" · ") || tr("等待登录后生成", "Generated after login") }}</p>
                  <p v-if="session.lastError" class="auth-session-error">{{ session.lastError }}</p>
                  <div class="auth-session-actions">
                    <button v-if="session.status === 'capturing'" class="button primary" type="button" :disabled="Boolean(authBusy)" @click="finishLogin(session)"><Check :size="14" />{{ tr("我已登录，完成捕获", "I'm signed in — finish") }}</button>
                    <button v-else class="button ghost" type="button" :disabled="Boolean(authBusy)" @click="openLogin(session)"><RefreshCw :size="13" />{{ tr("重新登录", "Sign in again") }}</button>
                    <button v-if="session.status !== 'capturing'" class="button ghost" type="button" :disabled="Boolean(authBusy)" @click="validateLogin(session)"><ShieldCheck :size="13" />{{ tr("校验", "Validate") }}</button>
                    <button class="icon-button danger" type="button" :disabled="Boolean(authBusy)" :title="tr('删除会话', 'Delete session')" @click="removeLogin(session)"><Trash2 :size="14" /></button>
                  </div>
                </article>
              </div>
              <div v-else class="auth-session-empty"><KeyRound :size="18" /><span>{{ tr("适合验证码、扫码、SSO 和多步骤登录。登录后先点开一个后台功能，再回来完成捕获，绿灯判定会更准确。", "Works with CAPTCHA, QR, SSO, and multi-step login. Visit one authenticated feature before finishing capture for a reliable green light.") }}</span></div>
              <footer><ShieldCheck :size="13" /><span>{{ tr("会话仅保存在本机 SQLite 和权限 600 的任务文件中；自动头由浏览器重建。单个 401/403 不熄灯，明确跳回登录页才判失效。", "Session data stays in local SQLite and mode-600 task files. Browser-managed headers are regenerated. One 401/403 does not invalidate the session; a clear redirect to login does.") }}</span></footer>
            </section>
            <section
              v-if="mode === 'greybox'"
              class="workbench-mode-settings span-two"
            >
              <header>
                <KeyRound :size="15" />
                <div>
                  <strong>灰盒环境与手动认证备用</strong
                  ><small
                    >优先使用上方浏览器会话；这里只为无法打开浏览器登录的 API Token 场景保留。</small
                  >
                </div>
              </header>
              <div class="workbench-fields nested-fields">
                <label class="field"
                  ><span>环境</span
                  ><input
                    v-model="form.environment"
                    placeholder="staging" /></label
                ><label class="field"
                  ><span>认证配置名称</span
                  ><input
                    v-model="form.authProfileName"
                    placeholder="测试管理员会话" /></label
                ><label class="field"
                  ><span>认证方式</span
                  ><select v-model="form.authType">
                    <option value="none">匿名</option>
                    <option value="cookie">Cookie</option>
                    <option value="bearer">Bearer Token</option>
                    <option value="header">自定义 Header</option>
                  </select></label
                ><label v-if="form.authType === 'header'" class="field"
                  ><span>Header 名称</span
                  ><input
                    v-model="form.authHeaderName"
                    placeholder="X-API-Key" /></label
                ><label v-if="form.authType !== 'none'" class="field span-two"
                  ><span>临时会话值</span
                  ><input
                    v-model="form.authValue"
                    type="password"
                    autocomplete="off"
                    :placeholder="
                      form.authType === 'cookie'
                        ? 'session=...'
                        : form.authType === 'bearer'
                          ? 'eyJ...'
                          : 'Header value'
                    "
                /></label>
              </div>
            </section>
            <section
              v-if="mode === 'cicd'"
              class="workbench-mode-settings span-two"
            >
              <header>
                <GitBranch :size="15" />
                <div>
                  <strong>流水线与发布门禁</strong
                  ><small
                    >CI/CD 只记录触发上下文并执行门禁，漏洞仍来自 SAST、SCA 和
                    Strix。</small
                  >
                </div>
              </header>
              <div class="workbench-fields nested-fields">
                <label class="field"
                  ><span>CI Provider</span
                  ><select v-model="form.ciProvider">
                    <option value="github">GitHub Actions</option>
                    <option value="gitlab">GitLab CI</option>
                    <option value="jenkins">Jenkins</option>
                    <option value="azure">Azure Pipelines</option>
                    <option value="other">Other</option>
                  </select></label
                ><label class="field"
                  ><span>仓库地址</span
                  ><input
                    v-model="form.repositoryUrl"
                    placeholder="https://github.com/org/repo" /></label
                ><label class="field"
                  ><span>分支</span
                  ><input v-model="form.branch" placeholder="main" /></label
                ><label class="field"
                  ><span>Commit SHA</span
                  ><input
                    v-model="form.commitSha"
                    placeholder="a89d20..." /></label
                ><label class="field"
                  ><span>Build / Pipeline ID</span
                  ><input
                    v-model="form.buildId"
                    placeholder="build-1024" /></label
                ><label class="field"
                  ><span>环境</span
                  ><input
                    v-model="form.environment"
                    placeholder="production" /></label
                ><label class="field"
                  ><span>允许 Critical 数</span
                  ><input
                    v-model.number="form.maxCritical"
                    type="number"
                    min="0" /></label
                ><label class="field"
                  ><span>允许 High 数</span
                  ><input
                    v-model.number="form.maxHigh"
                    type="number"
                    min="0" /></label
                ><label class="check-inline span-two"
                  ><input
                    v-model="form.blockRelease"
                    type="checkbox"
                  /><ShieldCheck :size="14" />超出阈值时阻断发布</label
                >
              </div>
            </section>
            <label class="field"
              ><span>{{ tr("扫描强度", "Scan mode") }}</span
              ><select v-model="form.scanMode">
                <option value="quick">Quick</option>
                <option value="standard">Standard</option>
                <option value="deep">Deep</option>
              </select></label
            >
            <label v-if="mode !== 'web'" class="field"
              ><span>{{ tr("代码范围", "Code scope") }}</span
              ><select v-model="form.scopeMode">
                <option value="auto">Auto</option>
                <option value="diff">Diff</option>
                <option value="full">Full</option>
              </select></label
            >
            <label
              v-if="mode !== 'web' && form.scopeMode !== 'full'"
              class="field"
              ><span>{{ tr("对比分支/提交", "Diff base") }}</span
              ><input v-model="form.diffBase" placeholder="origin/main"
            /></label>
            <label class="field"
              ><span>{{
                tr("费用上限（USD，可选）", "Budget cap (USD, optional)")
              }}</span
              ><input
                v-model.number="form.maxBudgetUsd"
                type="number"
                min="0.01"
                step="0.5"
                placeholder="—"
            /></label>
            <button
              v-if="mode === 'web'"
              type="button"
              class="workbench-advanced-toggle span-two"
              @click="showAdvanced = !showAdvanced"
            >
              <span>{{ showAdvanced ? '收起高级设置' : '展开高级设置' }}</span>
              <small>Skills、自定义调查要求；默认流程无需配置</small>
            </button>
            <div v-if="mode !== 'web' || showAdvanced" class="field span-two">
              <span>{{
                tr(
                  "本任务使用的 Skills（可多选，不会强制加载内置模板）",
                  "Skills for this task (multi-select; built-ins are not forced)",
                )
              }}</span>
              <div class="skill-choice-grid">
                <label
                  v-for="skill in skills.filter((item) => item.enabled)"
                  :key="skill.id"
                  :class="{ active: form.skillIds.includes(skill.id) }"
                  ><input
                    v-model="form.skillIds"
                    type="checkbox"
                    :value="skill.id"
                  /><span>{{ skill.name }}</span
                  ><small>{{
                    skill.description || tr("无说明", "No description")
                  }}</small
                  ><em>{{
                    skill.builtin
                      ? tr("内置", "Built-in")
                      : tr("自定义", "Custom")
                  }}</em></label
                >
                <div
                  v-if="!skills.some((item) => item.enabled)"
                  class="empty-inline"
                >
                  {{
                    tr(
                      "没有启用的 Skill，可在左侧 Skills 中创建",
                      "No enabled skills; create one from Skills",
                    )
                  }}
                </div>
              </div>
            </div>
            <label v-if="mode !== 'web' || showAdvanced" class="field span-two"
              ><span>{{
                tr("本次补充要求（可选）", "Extra instructions (optional)")
              }}</span
              ><textarea
                v-model="form.instruction"
                rows="3"
                :placeholder="
                  tr(
                    '例如：重点检查鉴权和文件上传',
                    'For example: focus on auth and file uploads',
                  )
                "
              ></textarea>
            </label>
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
                "Third-party libraries are inventoried and version-checked; business JS and app chunks receive deep analysis.",
              )
            }}</small
            ><button class="button primary" :disabled="busy" @click="start">
              <Zap :size="15" />{{
                busy
                  ? tr("启动中…", "Starting…")
                  : mode === 'web'
                    ? tr("启动自动调查", "Start investigation")
                    : tr("启动 Strix", "Start Strix")
              }}
            </button>
          </footer>
        </section>
        <aside class="workbench-side">
          <section class="panel token-by-feature">
            <div class="panel-heading compact">
              <div>
                <span class="eyebrow">TOKEN USAGE</span>
                <h3>{{ tr("按功能消耗", "Usage by feature") }}</h3>
              </div>
            </div>
            <div
              v-for="item in tokenByMode"
              :key="item.key"
              class="token-feature-row"
            >
              <header>
                <span>{{ modeLabel(item.key) }}</span>
                <strong>{{ format(item.total) }}</strong>
              </header>
              <dl>
                <div>
                  <dt>{{ tr("输入", "Input") }}</dt>
                  <dd>{{ format(item.input) }}</dd>
                </div>
                <div>
                  <dt>{{ tr("输出", "Output") }}</dt>
                  <dd>{{ format(item.output) }}</dd>
                </div>
                <div>
                  <dt>{{ tr("总计", "Total") }}</dt>
                  <dd>{{ format(item.total) }}</dd>
                </div>
              </dl>
            </div>
          </section>
          <section class="panel recent-workbench">
            <div class="panel-heading compact">
              <div>
                <span class="eyebrow">RECENT</span>
                <h3>{{ tr("最近任务", "Recent tasks") }}</h3>
              </div>
            </div>
            <button
              v-for="scan in modeScans.slice(0, 6)"
              :key="scan.id"
              @click="emit('openScan', scan)"
            >
              <span>{{ scan.taskName || scan.projectName }}</span
              ><small
                >{{ deploymentLabel(scan) }} · {{ scan.status }} · 输入 {{ format(scan.inputTokens) }} · 输出
                {{ format(scan.outputTokens) }} · 总计
                {{ format(scan.totalTokens) }}</small
              >
            </button>
            <div v-if="!modeScans.length" class="empty-state small">
              {{ tr("暂无此类任务", "No tasks of this type") }}
            </div>
          </section>
        </aside>
      </div>
    </template>

    <template v-else>
      <div class="skills-toolbar">
        <div>
          <span class="eyebrow">STRIX SKILLS</span>
          <h3>{{ tr("扫描技能", "Scan skills") }}</h3>
          <p>
            {{
              tr(
                "每个任务自行选择 Skill；代码审计不会再默认加载“业务前端深度分析”。",
                "Each task chooses its own skills; code audit no longer forces the frontend skill.",
              )
            }}
          </p>
        </div>
        <div>
          <button class="button ghost" @click="importSkills"><Upload :size="15" />{{ tr("导入", "Import") }}</button>
          <button class="button ghost" @click="importInternalSecSkills"><BookOpen :size="15" />{{ tr("导入内部 sec_skills", "Import internal sec_skills") }}</button>
          <button class="button ghost" @click="exportSkills"><Download :size="15" />{{ tr("导出", "Export") }}</button>
          <button class="button primary" @click="editSkill()"><Plus :size="15" />{{ tr("新增技能", "New skill") }}</button>
        </div>
      </div>
      <div class="skill-card-grid">
        <article
          v-for="skill in skills"
          :key="skill.id"
          class="panel skill-card-v2"
        >
          <header>
            <span :class="{ builtin: skill.builtin }">{{
              skill.builtin ? tr("内置", "Built-in") : tr("自定义", "Custom")
            }}</span
            ><em>{{
              skill.enabled ? tr("已启用", "Enabled") : tr("已停用", "Disabled")
            }}</em>
          </header>
          <h3>{{ skill.name }}</h3>
          <p>{{ skill.description }}</p>
          <details>
            <summary>{{ tr("查看扫描指令", "View instructions") }}</summary>
            <pre>{{ skill.instructions }}</pre>
          </details>
          <footer>
            <button
              class="button ghost compact"
              :disabled="refiningSkillId === skill.id"
              @click="refineSkill(skill)"
            >
              <Sparkles v-if="refiningSkillId !== skill.id" :size="13" />
              <RefreshCw v-else :size="13" class="spinning" />
              {{ refiningSkillId === skill.id ? tr("精炼中", "Refining") : tr("用最新知识精炼", "Refine with latest") }}
            </button>
            <button
              v-if="skill.builtin"
              class="button ghost compact"
              @click="cloneBuiltinSkill(skill)"
            >
              {{ tr("复制后编辑", "Clone & edit") }}
            </button>
            <button
              v-if="!skill.builtin"
              class="button ghost compact"
              @click="editSkill(skill)"
            >
              {{ tr("编辑", "Edit") }}</button
            ><button
              v-if="!skill.builtin"
              class="button danger compact"
              @click="deleteSkill = skill"
            >
              <Trash2 :size="13" />{{ tr("删除", "Delete") }}
            </button>
          </footer>
          <InlineConfirm
            v-if="deleteSkill?.id === skill.id"
            :title="
              tr(`删除技能「${skill.name}」？`, `Delete skill “${skill.name}”?`)
            "
            :detail="
              tr(
                '历史任务不受影响，新任务将不再加载该技能。',
                'Existing tasks are unchanged; new tasks will no longer load it.',
              )
            "
            :busy="skillBusy"
            @cancel="deleteSkill = undefined"
            @confirm="removeSkill"
          />
        </article>
      </div>
      <section v-if="showSkillEditor" class="panel skill-editor">
        <header>
          <div>
            <span class="eyebrow">SKILL EDITOR</span>
            <h3>
              {{
                editingSkill
                  ? tr("编辑自定义技能", "Edit custom skill")
                  : tr("新增自定义技能", "New custom skill")
              }}
            </h3>
          </div>
          <div>
            <button class="button ghost compact" @click="insertSkillTemplate">
              <Braces :size="14" />{{
                tr("插入规范模板", "Insert template")
              }}</button
            ><button class="icon-button" @click="showSkillEditor = false">
              <X :size="15" />
            </button>
          </div>
        </header>
        <div class="workbench-fields">
          <label class="field"
            ><span>{{ tr("名称", "Name") }}</span
            ><input
              v-model="skillForm.name"
              :placeholder="
                tr('例如：Java Spring 鉴权审计', 'e.g. Java Spring auth review')
              " /></label
          ><label class="field"
            ><span>{{ tr("说明", "Description") }}</span
            ><input
              v-model="skillForm.description"
              :placeholder="tr('一句话说明适用范围', 'One-line use case')"
          /></label>
          <div class="skill-format-guide span-two">
            <strong>{{ tr("推荐结构", "Recommended structure") }}</strong
            ><span
              >Objective → Scope → Analysis workflow → Output requirements</span
            >
            <p>
              {{
                tr(
                  "写清检查目标、包含/排除范围、验证步骤和输出字段。不要在 Skill 中要求跳过授权边界、批量破坏或把普通信息当漏洞。",
                  "Define objective, include/exclude scope, verification steps, and output fields. Do not request scope bypass, destructive bulk actions, or ordinary observations as vulnerabilities.",
                )
              }}
            </p>
          </div>
          <label class="field span-two"
            ><span>{{
              tr("传给 Strix 的指令", "Instructions sent to Strix")
            }}</span
            ><textarea
              v-model="skillForm.instructions"
              rows="16"
              :placeholder="
                tr('点击“插入规范模板”开始填写', 'Use Insert template to start')
              "
            ></textarea></label
          ><div class="skill-preview span-two">
            <header><strong>{{ tr("格式化预览", "Formatted preview") }}</strong><small>Markdown sections · {{ skillPreview.length }}</small></header>
            <article v-for="section in skillPreview" :key="section.heading"><h4>{{ section.heading }}</h4><pre>{{ section.body }}</pre></article>
            <p v-if="!skillPreview.length" class="empty-inline">{{ tr("输入指令后这里会显示章节化预览", "A sectioned preview appears as you type") }}</p>
          </div>
          ><label class="check-inline"
            ><input v-model="skillForm.enabled" type="checkbox" />{{
              tr("启用", "Enabled")
            }}</label
          >
        </div>
        <footer>
          <button class="button ghost" @click="showSkillEditor = false">
            {{ tr("取消", "Cancel") }}</button
          ><button
            class="button primary"
            :disabled="skillBusy"
            @click="saveSkill"
          >
            <Save :size="14" />{{ tr("保存技能", "Save skill") }}
          </button>
        </footer>
      </section>
    </template>
  </section>
</template>
