<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
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

const APP_LABELS: Record<string, [string, string]> = {
  "org.oviraptor.assets": ["资产", "Assets"],
  "org.oviraptor.scanning": ["扫描", "Scanning"],
};
const PERM_LABELS: Record<string, [string, string]> = {
  "inventory.read": ["读资产表", "Read inventory"],
  "inventory.exposure.read": ["读暴露面摘要", "Read exposure summary"],
  "investigation.read": ["读调查结果", "Read investigations"],
};

const activeProjects = computed(() => props.projects.filter(p => p.status === "active"));
const desktopProject = computed(() => activeProjects.value.find(p => p.id === desktopProjectId.value));
const recipientLabel = computed(() => {
  const id = settingsIdentity.value;
  if (!id) return "—";
  if (id.startsWith("xai-api")) return tr("Grok（OAuth）", "Grok (OAuth)");
  if (id.includes("grok")) return tr("Grok", "Grok");
  return id;
});

function appTitle(app: AppRow) {
  const mapped = APP_LABELS[app.id];
  if (mapped) return tr(...mapped);
  return app.manifest?.name || app.id.replace(/^org\.oviraptor\./, "");
}
function permTitle(id: string, description?: string) {
  const mapped = PERM_LABELS[id];
  if (mapped) return tr(...mapped);
  return description || id;
}

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
    emit("notify", "error", tr("请先确认只读授权说明", "Accept the disclosure first"));
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
  <div class="config-section data-permissions panel" data-testid="config-section-data-permissions">
    <div class="config-section-head">
      <h3>{{ tr("数据权限", "Data permissions") }}</h3>
    </div>

    <p v-if="error" class="host-action-error" role="alert">{{ error }}</p>
    <p v-else-if="!client" class="config-muted-line">{{ tr("Agent-host 不可用", "Agent-host unavailable") }}</p>
    <template v-else>
      <div class="config-form-row">
        <span>{{ tr("工作区", "Workspace") }}</span>
        <select v-model.number="desktopProjectId" data-testid="data-permissions-project">
          <option v-for="project in activeProjects" :key="project.id" :value="project.id">{{ project.name }}</option>
        </select>
      </div>
      <p class="config-inline-status">
        {{ tr("授权给", "Granted to") }} {{ recipientLabel }}
        <template v-if="desktopProject"> · {{ desktopProject.name }}</template>
      </p>
      <label class="disclosure">
        <input v-model="disclosureAccepted" type="checkbox" data-testid="data-permissions-disclosure" />
        <span>{{ tr("确认：仅授予所选只读权限，不含写入。", "Confirm: selected read-only grants only; no write access.") }}</span>
      </label>

      <article v-for="app in apps" :key="app.id" class="perm-block">
        <header>
          <div>
            <strong>{{ appTitle(app) }}</strong>
            <span class="meta">{{ app.id }}</span>
          </div>
        </header>
        <p v-if="!(app.manifest?.permissions?.length)" class="config-muted-line">{{ tr("此应用未声明权限", "No permissions declared") }}</p>
        <label v-for="permission in app.manifest?.permissions || []" :key="permission.id" class="perm-check">
          <input type="checkbox" :checked="(selected[app.id] || []).includes(permission.id)" @change="onPermissionChange(app.id, permission.id, $event)" />
          <span>
            <b>{{ permTitle(permission.id, permission.description) }}</b>
            <small>{{ permission.id }}</small>
          </span>
        </label>
        <div class="config-form-actions">
          <button class="button primary compact" type="button" :disabled="busy || !disclosureAccepted" @click="saveApp(app)">{{ tr("保存授权", "Save grant") }}</button>
        </div>
      </article>
      <p v-if="!apps.length" class="config-muted-line">{{ tr("请先在「已装应用」启用 Agent 工具", "Enable agent tools under Installed apps first") }}</p>
    </template>
  </div>
</template>
<style scoped>
.disclosure {
  display: flex;
  gap: 8px;
  align-items: flex-start;
  margin: 0 0 12px;
  font-size: 12px;
  line-height: 1.45;
  color: var(--app-ink, #172033);
}
.perm-block {
  margin-top: 8px;
  padding-top: 12px;
  border-top: 1px solid var(--line);
}
.perm-block header {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 8px;
}
.perm-block strong { font-size: 13px; }
.perm-block .meta {
  display: block;
  margin-top: 2px;
  font-size: 11px;
  color: var(--muted);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
.perm-check {
  display: flex;
  gap: 8px;
  align-items: flex-start;
  margin: 6px 0;
  font-size: 12px;
}
.perm-check b { display: block; font-weight: 550; }
.perm-check small {
  display: block;
  margin-top: 1px;
  font-size: 10px;
  color: var(--muted);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
.perm-block .config-form-actions { margin-top: 8px; }
</style>
