<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import AgentPermissionMode from "../../agentHost/components/AgentPermissionMode.vue";
import type { AgentAccessPolicy } from "../../agentHost/types";
import { ShieldCheck } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { Project } from "../../workspaces/types";
import { resolveAgentHostEndpoint } from "../../agentHost/api";
import { createAgentHostChatClient, type AgentHostChatClient } from "../../agentHost/chatClient";

type AppRow = {
  id: string;
  enabled: boolean;
  manifest?: {
    name?: string;
    permissions?: Array<{ id: string; description?: string }>;
  };
  grants?: Array<{ projectId: number; permissions: string[]; revision: number; disclosureRuntime?: string }>;
};

const props = defineProps<{ projects: Project[]; selectedProjectId?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();
const { tr } = useI18n();

const accessPolicy=ref<AgentAccessPolicy>();
const client = ref<AgentHostChatClient>();
const apps = ref<AppRow[]>([]);
const settingsIdentity = ref("");
const hostProjectId = ref<number>();
const desktopProjectId = ref<number | undefined>(props.selectedProjectId);
const selected = ref<Record<string, string[]>>({});
const revisions = ref<Record<string, number>>({});
const busy = ref(false);
const loading = ref(true);
const binding = ref(false);
const freshBinding = ref(false);
const error = ref("");
const disclosureAccepted = ref(false);
let generation = 0, loadGeneration = 0, disposed = false;

const activeProjects = computed(() => props.projects.filter(p => p.status === "active"));
const desktopProject = computed(() => activeProjects.value.find(p => p.id === desktopProjectId.value));
const canSave = computed(() => Boolean(client.value && desktopProject.value && hostProjectId.value !== undefined && settingsIdentity.value && freshBinding.value && !loading.value && !binding.value && !busy.value));
function clearScope() {
  hostProjectId.value = undefined; accessPolicy.value=undefined;
  selected.value = {}; revisions.value = {};
  disclosureAccepted.value = false; freshBinding.value = false;
}
const current = (epoch: number, projectId: number, c: AgentHostChatClient) => !disposed && epoch === generation && desktopProject.value?.id === projectId && client.value === c;
function permissionEffect(permission: string) {
  if (/(?:^|[._:-])(run|execute|exec)(?:$|[._:-])/i.test(permission)) return tr("执行", "Execute");
  if (/(?:^|[._:-])(write|delete|manage)(?:$|[._:-])/i.test(permission)) return tr("写入 / 修改", "Write / modify");
  if (/(?:^|[._:-])(read|list|search)(?:$|[._:-])/i.test(permission)) return tr("读取", "Read");
  return tr("应用声明的操作", "App-defined operation");
}

async function load() {
  const request = ++loadGeneration;
  generation++; clearScope(); binding.value = false; loading.value = true;
  client.value = undefined; apps.value = []; settingsIdentity.value = "";
  error.value = "";
  try {
    const endpoint = await resolveAgentHostEndpoint();
    if (disposed || request !== loadGeneration || !endpoint) return;
    const c = createAgentHostChatClient(endpoint);
    client.value = c;
    const [settings, listed] = await Promise.all([c.settings(), c.listApps()]);
    if (disposed || request !== loadGeneration) return;
    settingsIdentity.value = settings.runtime?.identity || "";
    apps.value = (listed.items ?? []).filter(app => app.enabled);
    loading.value = false;
    if (desktopProject.value) await bindProject(desktopProject.value);
  } catch (e) {
    if (!disposed && request === loadGeneration) { clearScope(); error.value = String((e as Error).message || e); }
  } finally { if (!disposed && request === loadGeneration) loading.value = false; }
}

async function bindProject(project: Project) {
  const c = client.value, epoch = ++generation;
  clearScope(); binding.value = true; error.value = "";
  if (!c) { binding.value = false; return false; }
  try {
    const id = await c.ensureProject(project.name, project.id);
    if (!current(epoch, project.id, c)) return false;
    const nextSelected: Record<string, string[]> = {}, nextRev: Record<string, number> = {};
    for (const app of apps.value) {
      const grant = (app.grants ?? []).find(g => g.projectId === id);
      nextSelected[app.id] = [...(grant?.permissions ?? [])];
      nextRev[app.id] = grant?.revision ?? 0;
    }
    hostProjectId.value = id; selected.value = nextSelected; revisions.value = nextRev;
    freshBinding.value = true;
    return true;
  } catch (e) {
    if (current(epoch, project.id, c)) error.value = String((e as Error).message || e);
    return false;
  } finally { if (current(epoch, project.id, c)) binding.value = false; }
}

function onPermissionChange(appId: string, permissionId: string, event: Event) {
  const target = event.target as HTMLInputElement | null;
  togglePermission(appId, permissionId, Boolean(target?.checked));
}
function togglePermission(appId: string, permissionId: string, on: boolean) {
  if (!canSave.value) return;
  const current = new Set(selected.value[appId] ?? []);
  if (on) current.add(permissionId); else current.delete(permissionId);
  selected.value = { ...selected.value, [appId]: [...current].sort() };
}

async function saveApp(app: AppRow) {
  if (!canSave.value || !client.value || !desktopProject.value || hostProjectId.value === undefined) return;
  if (!disclosureAccepted.value) {
    emit("notify", "error", tr("请先勾选权限披露确认", "Accept the disclosure first"));
    return;
  }
  const c = client.value, epoch = generation, project = desktopProject.value;
  const input = { projectId: hostProjectId.value, permissions: [...(selected.value[app.id] ?? [])], expectedRevision: revisions.value[app.id], disclosureAccepted: true as const, expectedRuntimeIdentity: settingsIdentity.value };
  busy.value = true;
  try {
    await c.setGrants(app.id, input);
    if (!current(epoch, project.id, c)) return;
    const listed = await c.listApps();
    if (!current(epoch, project.id, c)) return;
    apps.value = (listed.items ?? []).filter(a => a.enabled);
    if (await bindProject(project)) emit("notify", "success", tr("权限已保存", "Permissions saved"));
  } catch (e) {
    if (current(epoch, project.id, c)) { freshBinding.value = false; error.value = String((e as Error).message || e); emit("notify", "error", error.value); }
  } finally { busy.value = false; }
}

watch(() => [desktopProjectId.value, desktopProject.value?.id], () => {
  generation++; clearScope(); binding.value = false;
  if (!loading.value && desktopProject.value) void bindProject(desktopProject.value);
}, { flush: "sync" });
watch(() => props.selectedProjectId, id => { desktopProjectId.value = id; }, { flush: "sync" });
onMounted(async () => {
  if (!desktopProjectId.value && activeProjects.value[0]) desktopProjectId.value = activeProjects.value[0].id;
  await load();
});
onUnmounted(() => { disposed = true; generation++; loadGeneration++; clearScope(); });
</script>

<template>
  <div class="config-section data-permissions panel" data-testid="config-section-data-permissions" style="padding:22px">
    <header style="margin-bottom:16px">
      <h3><ShieldCheck :size="16" style="vertical-align:-2px;margin-right:6px" />{{ tr("数据权限", "Data permissions") }}</h3>
      <p class="helper" style="margin:6px 0 0;font-size:12px;color:var(--muted);line-height:1.6">
        {{ tr("按工作区授予应用声明的读取、写入或执行权限。请逐项核对操作范围；获准操作的数据与结果可发送给当前 Agent runtime。", "Grant app-declared read, write or execution permissions per workspace. Review each operation; its permitted data and results may be sent to the current Agent runtime.") }}
      </p>
    </header>
    <button class="button ghost compact" type="button" :disabled="loading || busy || binding" @click="load">{{ tr('刷新权限', 'Refresh permissions') }}</button>
    <p v-if="error" class="host-action-error" role="alert">{{ error }}</p>
    <p v-if="loading || binding" class="helper" role="status">{{ tr('正在核对所选工作区与授权版本…', 'Checking the selected workspace and grant revision…') }}</p>
    <p v-else-if="!client" class="helper">{{ tr("Agent-host sidecar 不可用。请先确认桌面已启动 sidecar。", "Agent-host sidecar unavailable.") }}</p>
    <template v-else>
      <label class="field" style="max-width:360px;display:block;margin-bottom:14px">
        <span>{{ tr("工作区", "Workspace") }}</span>
        <select v-model.number="desktopProjectId" :disabled="loading || busy" data-testid="data-permissions-project">
          <option v-for="project in activeProjects" :key="project.id" :value="project.id">{{ project.name }}</option>
        </select>
      </label>
      <p v-if="!desktopProject" class="helper">{{ tr('请选择一个未归档的工作区，再核对和保存授权。', 'Select an active workspace before reviewing and saving grants.') }}</p>
      <p class="helper" style="font-size:12px;margin-bottom:12px">
        {{ tr("Runtime 接收方", "Runtime recipient") }}：<code>{{ settingsIdentity || "—" }}</code>
        <template v-if="hostProjectId"> · host project #{{ hostProjectId }}</template>
      </p>
      <AgentPermissionMode v-if="client&&hostProjectId" :client="client" :project-id="hostProjectId" :runtime-identity="settingsIdentity" :disabled="busy||binding||loading" @changed="accessPolicy=$event" />
      <p v-if="accessPolicy?.mode==='full'" class="helper">{{tr('完全放开已生效：无需逐项保存。下方原有精细授权保持保存，切回询问模式后继续使用。新安装应用需重新确认。','Full access is active; no individual saves are needed. Existing fine-grained grants are retained for Ask mode. New apps require confirmation.')}}</p>
      <template v-if="accessPolicy?.mode!=='full'">
      <label class="check-field" style="display:flex;gap:8px;align-items:flex-start;margin-bottom:16px;font-size:12px;line-height:1.5">
        <input v-model="disclosureAccepted" type="checkbox" :disabled="!canSave" data-testid="data-permissions-disclosure" />
        <span>{{ tr("我已核对所选工作区与各项权限，允许当前 Agent runtime 接收获准操作的数据与结果；勾选写入或执行权限会允许对应操作。", "I reviewed this workspace and each permission, and allow the current Agent runtime to receive permitted operation data and results. Selected write or execution permissions allow those operations.") }}</span>
      </label>
      <article v-for="app in apps" :key="app.id" style="border-top:1px solid var(--line);padding:14px 0">
        <div style="display:flex;justify-content:space-between;gap:12px;align-items:center;margin-bottom:8px">
          <div>
            <strong>{{ app.manifest?.name || app.id }}</strong>
            <small style="display:block;color:var(--muted)">{{ app.id }}</small>
          </div>
          <button class="button primary compact" type="button" :disabled="!canSave || !disclosureAccepted" @click="saveApp(app)">{{ tr("保存授权", "Save grant") }}</button>
        </div>
        <div v-if="!(app.manifest?.permissions?.length)" class="helper">{{ tr("此应用未声明权限。", "This app declares no permissions.") }}</div>
        <label v-for="permission in app.manifest?.permissions || []" :key="permission.id" style="display:flex;gap:8px;align-items:flex-start;margin:6px 0;font-size:12px">
          <input type="checkbox" :checked="(selected[app.id] || []).includes(permission.id)" :disabled="!canSave" @change="onPermissionChange(app.id, permission.id, $event)" />
          <span><strong>{{ permissionEffect(permission.id) }}</strong> · <code>{{ permission.id }}</code> — {{ permission.description || permission.id }}</span>
        </label>
      </article>
      </template>
      <p v-if="!apps.length" class="helper">{{ tr("没有已启用的 Agent 应用。请先到「已装应用」启用 assets / scanning。", "No enabled agent apps. Enable assets / scanning under Installed apps first.") }}</p>
    </template>
  </div>
</template>
