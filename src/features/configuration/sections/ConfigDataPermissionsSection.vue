<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import AgentPermissionMode from "../../agentHost/components/AgentPermissionMode.vue";
import AgentToolPermissionRulesPanel from "../../sentinel/components/AgentToolPermissionRulesPanel.vue";
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
  <div class="config-section data-permissions" data-testid="config-section-data-permissions">
    <header class="permissions-heading">
      <div>
        <h3><ShieldCheck :size="17" />{{ tr("应用授权", "App permissions") }}</h3>
        <p>{{ tr("按工作区管理应用的读取、写入与执行权限。", "Manage app read, write and execution permissions per workspace.") }}</p>
      </div>
      <button v-refresh-feedback="{scope:desktopProjectId,busy:loading,error}" class="button ghost compact" type="button" :disabled="loading || busy || binding" @click="load">{{ tr('刷新权限', 'Refresh permissions') }}</button>
    </header>
    <p v-if="error" class="permissions-error" role="alert">{{ error }}</p>
    <p v-if="loading || binding" class="permissions-status" role="status">{{ tr('正在核对所选工作区与授权版本…', 'Checking the selected workspace and grant revision…') }}</p>
    <p v-else-if="!client" class="permissions-status">{{ tr("Agent-host sidecar 不可用。请先确认桌面已启动 sidecar。", "Agent-host sidecar unavailable.") }}</p>
    <template v-else>
      <div class="permissions-context">
        <label class="field permissions-project">
          <span>{{ tr("工作区", "Workspace") }}</span>
          <select v-model.number="desktopProjectId" :disabled="loading || busy" data-testid="data-permissions-project">
            <option v-for="project in activeProjects" :key="project.id" :value="project.id">{{ project.name }}</option>
          </select>
        </label>
        <div class="permissions-recipient">
          <span>{{ tr("Runtime 接收方", "Runtime recipient") }}</span>
          <div><code>{{ settingsIdentity || "—" }}</code><small v-if="hostProjectId">host project #{{ hostProjectId }}</small></div>
        </div>
      </div>
      <p v-if="!desktopProject" class="permissions-status">{{ tr('请选择一个未归档的工作区，再核对和保存授权。', 'Select an active workspace before reviewing and saving grants.') }}</p>
      <AgentPermissionMode v-if="client&&hostProjectId" class="permissions-mode" :client="client" :project-id="hostProjectId" :runtime-identity="settingsIdentity" :disabled="busy||binding||loading" @changed="accessPolicy=$event" />
      <AgentToolPermissionRulesPanel @notify="(type, text) => emit('notify', type, text)" />
      <p v-if="accessPolicy?.mode==='full'" class="permissions-full-notice">{{tr('完全放开已生效：无需逐项保存。原有精细授权保持保存，切回询问模式后继续使用。新安装应用需重新确认。','Full access is active; no individual saves are needed. Existing fine-grained grants are retained for Ask mode. New apps require confirmation.')}}</p>
      <template v-if="accessPolicy?.mode!=='full'">
        <label class="permissions-disclosure">
          <input v-model="disclosureAccepted" type="checkbox" :disabled="!canSave" data-testid="data-permissions-disclosure" />
          <span><strong>{{ tr("授权确认", "Confirm access") }}</strong>{{ tr("我已核对所选工作区与各项权限，允许当前 Agent runtime 接收获准操作的数据与结果；勾选写入或执行权限会允许对应操作。", "I reviewed this workspace and each permission, and allow the current Agent runtime to receive permitted operation data and results. Selected write or execution permissions allow those operations.") }}</span>
        </label>
        <div class="permission-app-list">
          <article v-for="app in apps" :key="app.id" class="permission-app">
            <header class="permission-app-heading">
              <div class="permission-app-identity">
                <h4>{{ app.manifest?.name || app.id }}</h4>
                <code v-if="app.manifest?.name && app.manifest.name !== app.id">{{ app.id }}</code>
                <span>{{ tr('已选', 'Selected') }} {{ (app.manifest?.permissions || []).filter(permission => (selected[app.id] || []).includes(permission.id)).length }} / {{ app.manifest?.permissions?.length || 0 }}</span>
              </div>
              <button class="button primary compact" type="button" :disabled="!canSave || !disclosureAccepted" @click="saveApp(app)">{{ tr("保存授权", "Save grant") }}</button>
            </header>
            <p v-if="!(app.manifest?.permissions?.length)" class="permissions-empty">{{ tr("此应用未声明权限。", "This app declares no permissions.") }}</p>
            <div v-for="permission in app.manifest?.permissions || []" :key="permission.id" class="permission-row" :data-permission-id="permission.id">
              <label class="permission-choice">
                <input type="checkbox" :checked="(selected[app.id] || []).includes(permission.id)" :disabled="!canSave" @change="onPermissionChange(app.id, permission.id, $event)" />
                <span class="permission-effect">{{ permissionEffect(permission.id) }}</span>
                <code>{{ permission.id }}</code>
              </label>
              <details v-if="permission.description && permission.description !== permission.id && permission.description.length > 140" class="permission-description">
                <summary><span class="permission-preview">{{ permission.description }}</span><span class="permission-expand">{{ tr('完整说明', 'Full description') }}</span><span class="permission-collapse">{{ tr('收起说明', 'Hide description') }}</span></summary>
                <p>{{ permission.description }}</p>
              </details>
              <p v-else-if="permission.description && permission.description !== permission.id" class="permission-short-description">{{ permission.description }}</p>
            </div>
          </article>
        </div>
      </template>
      <p v-if="!apps.length" class="permissions-status">{{ tr("没有已启用的 Agent 应用。请先到「已装应用」启用 assets / scanning。", "No enabled agent apps. Enable assets / scanning under Installed apps first.") }}</p>
    </template>
  </div>
</template>

<style scoped>
.data-permissions{container-type:inline-size;min-width:0;color:var(--ui-ink);font:var(--text-body)/1.6 var(--font-sans)}
.permissions-heading{display:flex;align-items:center;justify-content:space-between;gap:16px;margin-bottom:20px}.permissions-heading>div{min-width:0}.permissions-heading h3{display:flex;align-items:center;gap:8px;margin:0;font-size:var(--text-heading);font-weight:600;line-height:1.5}.permissions-heading h3 svg{flex-shrink:0;color:var(--ui-muted)}.permissions-heading p{margin:4px 0 0;color:var(--ui-muted);font-size:var(--text-meta)}
.data-permissions .button{flex-shrink:0;min-height:36px;padding:8px 13px;font-size:var(--text-body);white-space:nowrap}.permissions-status{margin:12px 0;color:var(--ui-muted);font-size:var(--text-meta);overflow-wrap:anywhere}.permissions-error{margin:0 0 16px;padding:10px 12px;border:1px solid color-mix(in srgb,var(--red) 35%,var(--ui-line));border-radius:var(--radius-md);background:color-mix(in srgb,var(--red) 5%,var(--ui-surface));color:color-mix(in srgb,var(--red) 82%,var(--ui-ink));overflow-wrap:anywhere;white-space:pre-wrap}
.permissions-context{display:grid;grid-template-columns:minmax(180px,320px) minmax(0,1fr);align-items:start;gap:20px;margin-bottom:18px}.data-permissions .permissions-project{display:flex;flex-direction:column;gap:6px;min-width:0;margin:0}.permissions-project>span,.permissions-recipient>span{display:block;color:var(--ui-muted);font-size:var(--text-meta);font-weight:600;line-height:1.4}.permissions-project select{box-sizing:border-box;width:100%;min-width:0;height:38px;padding:0 10px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface);color:var(--ui-ink);font:var(--text-body)/1.5 var(--font-sans)}
.permissions-recipient{min-width:0}.permissions-recipient>div{display:flex;align-items:center;flex-wrap:wrap;gap:6px 12px;min-height:38px;margin-top:6px}.permissions-recipient code{font:var(--text-meta)/1.6 var(--font-mono);overflow-wrap:anywhere}.permissions-recipient small{font-size:var(--text-meta);color:var(--ui-muted)}.permissions-mode{padding:16px 0;border-top:1px solid var(--ui-line)}
.permissions-full-notice{margin:0;padding:12px 14px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised);font-size:var(--text-meta);line-height:1.7;overflow-wrap:anywhere}
.permissions-disclosure{display:flex;align-items:flex-start;gap:10px;padding:12px 14px;margin:0 0 20px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised);cursor:pointer;font-size:var(--text-meta);line-height:1.7}.permissions-disclosure strong{display:block;margin-bottom:2px;font-size:var(--text-body);font-weight:600}.data-permissions input[type=checkbox]{flex-shrink:0;appearance:auto;width:15px;height:15px;margin:3px 0 0;accent-color:var(--ui-accent);cursor:pointer}.data-permissions input:disabled{cursor:default}
.permission-app-list{display:grid;gap:20px}.permission-app{min-width:0;border:1px solid var(--ui-line);border-radius:var(--radius-lg);background:var(--ui-surface);overflow:hidden}.permission-app-heading{display:flex;align-items:center;justify-content:space-between;gap:16px;padding:14px 16px;border-bottom:1px solid var(--ui-line);background:var(--ui-surface-raised)}.permission-app-identity{display:flex;align-items:center;flex-wrap:wrap;gap:4px 10px;min-width:0}.permission-app-identity h4{margin:0;font-size:var(--text-strong);font-weight:600;overflow-wrap:anywhere}.permission-app-identity>code{color:var(--ui-muted);font:var(--text-meta)/1.6 var(--font-mono);overflow-wrap:anywhere}.permission-app-identity>span{font-size:var(--text-meta);color:var(--ui-muted);white-space:nowrap}.permission-row{min-width:0;padding:11px 16px;border-bottom:1px solid var(--ui-line)}.permission-row:last-child{border-bottom:0}.permission-choice{display:grid;grid-template-columns:15px auto minmax(0,1fr);align-items:start;gap:9px;cursor:pointer}.permission-effect{display:inline-block;padding:1px 7px;border-radius:var(--radius-sm);background:var(--ui-surface-sunken);font-size:var(--text-micro);line-height:1.7;white-space:nowrap;color:var(--ui-copy,var(--ui-ink))}.permission-choice code{padding-top:1px;font:var(--text-meta)/1.7 var(--font-mono);overflow-wrap:anywhere}.permission-short-description,.permission-description{margin:5px 0 0 24px;color:var(--ui-muted);font-size:var(--text-meta);line-height:1.65;overflow-wrap:anywhere}.permission-short-description{margin-bottom:0}
.permission-description>summary{display:flex;align-items:flex-start;gap:12px;list-style:none;cursor:pointer}.permission-description>summary::-webkit-details-marker{display:none}.permission-preview{display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;overflow:hidden;flex:1;min-width:0}.permission-expand,.permission-collapse{flex-shrink:0;color:var(--ui-accent);font-size:var(--text-meta);white-space:nowrap}.permission-collapse{display:none}.permission-description[open] .permission-preview,.permission-description[open] .permission-expand{display:none}.permission-description[open] .permission-collapse{display:inline}.permission-description>p{margin:6px 0 0;color:var(--ui-copy,var(--ui-ink));white-space:pre-wrap}.permissions-empty{padding:14px 16px;margin:0;color:var(--ui-muted);font-size:var(--text-meta)}
.data-permissions :is(select,input,summary):focus-visible{outline:2px solid var(--ui-accent);outline-offset:3px}
@container(max-width:600px){.permissions-context{grid-template-columns:minmax(0,1fr);gap:14px}.permissions-recipient>div{min-height:0}.permission-app-heading{padding:12px}.permission-row{padding:10px 12px}.permission-app-identity{flex-direction:column;align-items:flex-start;gap:3px}.permission-choice{grid-template-columns:15px minmax(0,1fr);gap:4px 9px}.permission-choice input{grid-row:1/3}.permission-effect{justify-self:start}.permission-choice code{grid-column:2}.permission-description>summary{flex-direction:column;gap:4px}.permissions-heading{align-items:flex-start}.permissions-disclosure{padding:10px 12px}}
</style>
