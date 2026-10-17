<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
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

const client = ref<AgentHostChatClient>();
const apps = ref<AppRow[]>([]);
const settingsIdentity = ref("");
const hostProjectId = ref<number>();
const desktopProjectId = ref<number | undefined>(props.selectedProjectId);
const selected = ref<Record<string, string[]>>({});
const revisions = ref<Record<string, number>>({});
const busy = ref(false);
const error = ref("");
const disclosureAccepted = ref(false);

const activeProjects = computed(() => props.projects.filter(p => p.status === "active"));
const desktopProject = computed(() => activeProjects.value.find(p => p.id === desktopProjectId.value));

async function load() {
  error.value = "";
  try {
    const endpoint = await resolveAgentHostEndpoint();
    if (!endpoint) { client.value = undefined; apps.value = []; return; }
    const c = createAgentHostChatClient(endpoint);
    client.value = c;
    const settings = await c.settings();
    settingsIdentity.value = settings.runtime?.identity || "";
    const listed = await c.listApps();
    apps.value = (listed.items ?? []).filter(app => app.enabled);
    if (desktopProject.value) await bindProject(desktopProject.value);
  } catch (e) {
    error.value = String((e as Error).message || e);
  }
}

async function bindProject(project: Project) {
  if (!client.value) return;
  hostProjectId.value = await client.value.ensureProject(project.name, project.id);
  const nextSelected: Record<string, string[]> = {};
  const nextRev: Record<string, number> = {};
  for (const app of apps.value) {
    const grant = (app.grants ?? []).find(g => g.projectId === hostProjectId.value);
    nextSelected[app.id] = [...(grant?.permissions ?? [])];
    nextRev[app.id] = grant?.revision ?? 0;
  }
  selected.value = nextSelected;
  revisions.value = nextRev;
  disclosureAccepted.value = false;
}

function onPermissionChange(appId: string, permissionId: string, event: Event) {
  const target = event.target as HTMLInputElement | null;
  togglePermission(appId, permissionId, Boolean(target?.checked));
}
function togglePermission(appId: string, permissionId: string, on: boolean) {
  const current = new Set(selected.value[appId] ?? []);
  if (on) current.add(permissionId); else current.delete(permissionId);
  selected.value = { ...selected.value, [appId]: [...current].sort() };
}

async function saveApp(app: AppRow) {
  if (!client.value || hostProjectId.value === undefined || !settingsIdentity.value) return;
  if (!disclosureAccepted.value) {
    emit("notify", "error", tr("请先勾选权限披露确认", "Accept the disclosure first"));
    return;
  }
  busy.value = true;
  try {
    await client.value.setGrants(app.id, {
      projectId: hostProjectId.value,
      permissions: selected.value[app.id] ?? [],
      expectedRevision: revisions.value[app.id],
      disclosureAccepted: true,
      expectedRuntimeIdentity: settingsIdentity.value,
    });
    const listed = await client.value.listApps();
    apps.value = (listed.items ?? []).filter(a => a.enabled);
    if (desktopProject.value) await bindProject(desktopProject.value);
    emit("notify", "success", tr("权限已保存", "Permissions saved"));
  } catch (e) {
    emit("notify", "error", String((e as Error).message || e));
  } finally { busy.value = false; }
}

watch(desktopProjectId, async (id) => {
  const project = activeProjects.value.find(p => p.id === id);
  if (project) await bindProject(project);
});
onMounted(async () => {
  if (!desktopProjectId.value && activeProjects.value[0]) desktopProjectId.value = activeProjects.value[0].id;
  await load();
});
</script>

<template>
  <div class="config-section data-permissions panel" data-testid="config-section-data-permissions" style="padding:22px">
    <header style="margin-bottom:16px">
      <h3><ShieldCheck :size="16" style="vertical-align:-2px;margin-right:6px" />{{ tr("数据权限", "Data permissions") }}</h3>
      <p class="helper" style="margin:6px 0 0;font-size:12px;color:var(--muted);line-height:1.6">
        {{ tr("按工作区授予 Agent 只读工具权限（inventory.read / investigation.read / inventory.exposure.read 等）。接收方是当前 Agent runtime 身份；未授权时 Agent 不得读取该项目数据。", "Grant per-workspace read tools (inventory.read / investigation.read / inventory.exposure.read). Recipient is the current Agent runtime identity; without a grant the agent cannot read that project.") }}
      </p>
    </header>
    <p v-if="error" class="host-action-error" role="alert">{{ error }}</p>
    <p v-else-if="!client" class="helper">{{ tr("Agent-host sidecar 不可用。请先确认桌面已启动 sidecar。", "Agent-host sidecar unavailable.") }}</p>
    <template v-else>
      <label class="field" style="max-width:360px;display:block;margin-bottom:14px">
        <span>{{ tr("工作区", "Workspace") }}</span>
        <select v-model.number="desktopProjectId" data-testid="data-permissions-project">
          <option v-for="project in activeProjects" :key="project.id" :value="project.id">{{ project.name }}</option>
        </select>
      </label>
      <p class="helper" style="font-size:12px;margin-bottom:12px">
        {{ tr("Runtime 接收方", "Runtime recipient") }}：<code>{{ settingsIdentity || "—" }}</code>
        <template v-if="hostProjectId"> · host project #{{ hostProjectId }}</template>
      </p>
      <label class="check-field" style="display:flex;gap:8px;align-items:flex-start;margin-bottom:16px;font-size:12px;line-height:1.5">
        <input v-model="disclosureAccepted" type="checkbox" data-testid="data-permissions-disclosure" />
        <span>{{ tr("我理解所选权限会把该工作区的获准只读数据发送给当前 Agent runtime，不会自动授予写权限。", "I understand these permissions send permitted read-only workspace data to the current Agent runtime and do not grant write access.") }}</span>
      </label>
      <article v-for="app in apps" :key="app.id" style="border-top:1px solid var(--line);padding:14px 0">
        <div style="display:flex;justify-content:space-between;gap:12px;align-items:center;margin-bottom:8px">
          <div>
            <strong>{{ app.manifest?.name || app.id }}</strong>
            <small style="display:block;color:var(--muted)">{{ app.id }}</small>
          </div>
          <button class="button primary compact" type="button" :disabled="busy || !disclosureAccepted" @click="saveApp(app)">{{ tr("保存授权", "Save grant") }}</button>
        </div>
        <div v-if="!(app.manifest?.permissions?.length)" class="helper">{{ tr("此应用未声明权限。", "This app declares no permissions.") }}</div>
        <label v-for="permission in app.manifest?.permissions || []" :key="permission.id" style="display:flex;gap:8px;align-items:flex-start;margin:6px 0;font-size:12px">
          <input type="checkbox" :checked="(selected[app.id] || []).includes(permission.id)" @change="onPermissionChange(app.id, permission.id, $event)" />
          <span><code>{{ permission.id }}</code> — {{ permission.description || permission.id }}</span>
        </label>
      </article>
      <p v-if="!apps.length" class="helper">{{ tr("没有已启用的 Agent 应用。请先到「已装应用」启用 assets / scanning。", "No enabled agent apps. Enable assets / scanning under Installed apps first.") }}</p>
    </template>
  </div>
</template>
