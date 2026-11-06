<script setup lang="ts">
import { onMounted, ref } from "vue";
import {
  Activity,
  CheckCircle2,
  CircleAlert,
  Copy,
  Laptop,
  Pause,
  Play,
  Plus,
  RefreshCw,
  Server,
  Square,
  Trash2,
} from "@lucide/vue";
import { runtimeApi } from "../api";
import { remoteScanningApi } from "../../sentinel/remoteWorkerApi";
import type { EnvironmentReport } from "../../environment/types";
import type { LocalWorkerSettings, RemoteWorkerNode, WorkerHealth } from "../types";
import type { SentinelScan } from "../../sentinel/types";
import { useI18n } from "../../../i18n";

const emit = defineEmits<{
  message: [type: "success" | "error" | "info", text: string];
}>();
const { tr } = useI18n();

const local = ref<LocalWorkerSettings>();
const nodes = ref<RemoteWorkerNode[]>([]);
const loading = ref(true);
const savingLocal = ref(false);
const busyNode = ref<number>();
const form = ref({ name: "", endpoint: "", accessToken: "" });
const health = ref<Record<number, WorkerHealth>>({});
const environments = ref<Record<number, EnvironmentReport>>({});
const scans = ref<Record<number, SentinelScan[]>>({});

function dependencyStatus(report: EnvironmentReport, name: string) {
  return report.dependencies.find((item) => item.name.toLowerCase() === name.toLowerCase());
}

function message(type: "success" | "error" | "info", text: string) {
  emit("message", type, text);
}

async function load() {
  loading.value = true;
  try {
    [local.value, nodes.value] = await Promise.all([
      runtimeApi.getLocalWorkerSettings(),
      runtimeApi.listWorkerNodes(),
    ]);
  } catch (error) {
    message("error", String(error));
  } finally {
    loading.value = false;
  }
}

async function saveLocal() {
  if (!local.value) return;
  savingLocal.value = true;
  try {
    local.value = await runtimeApi.saveLocalWorkerSettings({
      enabled: local.value.enabled,
      port: Number(local.value.port),
      accessToken: local.value.accessToken,
    });
    message("success", local.value.status);
  } catch (error) {
    message("error", String(error));
  } finally {
    savingLocal.value = false;
  }
}

async function copy(value: string, label: string) {
  try {
    await navigator.clipboard.writeText(value);
    message("success", tr(`${label}已复制`, `${label} copied`));
  } catch {
    message("error", tr(`无法复制${label}，请手动选择复制`, `Could not copy the ${label.toLowerCase()}; select and copy it manually`));
  }
}

async function addNode() {
  try {
    const id = await runtimeApi.saveWorkerNode({
      ...form.value,
      enabled: true,
    });
    form.value = { name: "", endpoint: "", accessToken: "" };
    await load();
    message("success", tr(`Worker 节点 #${id} 已保存`, `Worker node #${id} saved`));
  } catch (error) {
    message("error", String(error));
  }
}

async function removeNode(node: RemoteWorkerNode) {
  if (busyNode.value) return;
  busyNode.value = node.id;
  try {
    await runtimeApi.deleteWorkerNode(node.id);
    await load();
    message("success", tr(`${node.name} 已从主控端移除；Worker 本机数据未删除`, `${node.name} removed from the controller; data on the Worker was not deleted`));
  } catch (error) {
    message("error", String(error));
  } finally {
    busyNode.value = undefined;
  }
}

async function inspectNode(node: RemoteWorkerNode) {
  busyNode.value = node.id;
  try {
    const [nodeHealth, nodeEnvironment, nodeScans] = await Promise.all([
      runtimeApi.testWorkerNode(node.id),
      runtimeApi.getRemoteWorkerEnvironment(node.id),
      remoteScanningApi.listRemoteWorkerScans(node.id),
    ]);
    health.value[node.id] = nodeHealth;
    environments.value[node.id] = nodeEnvironment;
    scans.value[node.id] = nodeScans;
    message("success", tr(`${node.name} 连接正常，环境与任务状态已刷新`, `${node.name} is reachable; environment and task status refreshed`));
    nodes.value = await runtimeApi.listWorkerNodes();
  } catch (error) {
    message("error", String(error));
    nodes.value = await runtimeApi.listWorkerNodes().catch(() => nodes.value);
  } finally {
    busyNode.value = undefined;
  }
}

async function syncNode(node: RemoteWorkerNode) {
  busyNode.value = node.id;
  try {
    const count = await remoteScanningApi.syncWorkerNode(node.id);
    message("success", tr(`${node.name} 已读取 ${count} 条只读历史记录，可在任务中心的历史产物中查看；不会创建或恢复本地扫描任务，已删除或较旧轮次可能不显示。`, `Read ${count} read-only history records from ${node.name}. View them under history artifacts in the Task Center. No local scan tasks are created or resumed; deleted or older rounds may not appear.`));
    nodes.value = await runtimeApi.listWorkerNodes();
  } catch (error) {
    message("error", String(error));
  } finally {
    busyNode.value = undefined;
  }
}

async function control(
  node: RemoteWorkerNode,
  scan: SentinelScan,
  action: "pause" | "resume" | "cancel",
) {
  busyNode.value = node.id;
  try {
    await remoteScanningApi.controlRemoteWorkerScan({
      nodeId: node.id,
      scanId: scan.id,
      action,
    });
    scans.value[node.id] = await remoteScanningApi.listRemoteWorkerScans(node.id);
    message("success", action === "pause" ? tr("远程任务已暂停", "Remote task paused") : action === "resume" ? tr("远程任务已继续", "Remote task resumed") : tr("远程任务已取消", "Remote task cancelled"));
  } catch (error) {
    message("error", String(error));
  } finally {
    busyNode.value = undefined;
  }
}

onMounted(load);
</script>

<template>
  <div class="worker-settings">
    <section class="panel local-worker">
      <header class="worker-section-heading">
        <div>
          <h3>{{ tr("本机 Worker", "Local Worker") }}</h3>
          <p>{{ tr("服务只监听 Tailscale 私网地址；本机可同时作为主控端和 Worker。", "The service listens only on the Tailscale private address; this machine can be both the controller and a Worker.") }}</p>
        </div>
        <span v-if="local" class="worker-state" :class="{ online: local.running }">
          {{ local.running ? tr("运行中", "Running") : tr("未运行", "Not running") }}
        </span>
      </header>
      <div v-if="loading" class="worker-empty" role="status">{{ tr("正在读取 Worker 设置…", "Loading Worker settings…") }}</div>
      <template v-else-if="local">
        <div class="worker-form local-form">
          <label class="worker-toggle">
            <input v-model="local.enabled" type="checkbox" />
            <span aria-hidden="true"></span>
            <b>{{ tr("启用本机 Worker", "Enable local Worker") }}</b>
          </label>
          <label>
            <span>{{ tr("监听端口", "Listening port") }}</span>
            <input v-model.number="local.port" type="number" min="1024" max="65535" />
          </label>
          <button class="button primary" type="button" :disabled="savingLocal" @click="saveLocal">
            <RefreshCw v-if="savingLocal" :size="14" class="spinning" />
            {{ savingLocal ? tr("正在启动…", "Starting…") : tr("保存并重启 Worker", "Save and restart Worker") }}
          </button>
        </div>
        <p class="worker-status" :class="{ error: local.enabled && !local.running }" role="status">{{ local.status }}</p>
        <div class="connection-values">
          <div class="connection-value">
            <span>{{ tr("Tailnet 地址", "Tailnet address") }}</span>
            <code>{{ local.endpoint || tr("尚未检测到 Tailscale IPv4", "No Tailscale IPv4 detected yet") }}</code>
            <button v-if="local.endpoint" type="button" class="icon-button" :title="tr('复制节点地址', 'Copy node address')" :aria-label="tr('复制节点地址', 'Copy node address')" @click="copy(local.endpoint, tr('节点地址', 'Node address'))"><Copy :size="14" /></button>
          </div>
          <div class="connection-value">
            <span>{{ tr("访问令牌", "Access token") }}</span>
            <code>{{ local.accessToken }}</code>
            <button type="button" class="icon-button" :title="tr('复制访问令牌', 'Copy access token')" :aria-label="tr('复制访问令牌', 'Copy access token')" @click="copy(local.accessToken, tr('访问令牌', 'Access token'))"><Copy :size="14" /></button>
          </div>
        </div>
        <p class="worker-lifecycle-note">{{ tr("关闭窗口后应用留在任务栏；选择“退出应用”才会停止 Worker。", "Closing the window keeps the app in the taskbar; only “Quit app” stops the Worker.") }}</p>
      </template>
    </section>

    <section class="panel add-worker">
      <header class="worker-section-heading">
        <div><h3>{{ tr("添加远程 Worker", "Add remote Worker") }}</h3><p>{{ tr("填写远程电脑上显示的节点地址和访问令牌。", "Enter the node address and access token shown on the remote computer.") }}</p></div>
      </header>
      <div class="worker-form node-form">
        <label><span>{{ tr("名称", "Name") }}</span><input v-model="form.name" :placeholder="tr('例如：家中 Intel Mac Pro', 'e.g. Intel Mac Pro at home')" /></label>
        <label><span>{{ tr("节点地址", "Node address") }}</span><input v-model="form.endpoint" placeholder="http://100.x.x.x:19427" /></label>
        <label><span>{{ tr("访问令牌", "Access token") }}</span><input v-model="form.accessToken" type="password" :placeholder="tr('粘贴 Worker 令牌', 'Paste the Worker token')" /></label>
        <button class="button primary" type="button" @click="addNode"><Plus :size="14" />{{ tr("保存节点", "Save node") }}</button>
      </div>
    </section>

    <section class="worker-node-list" :aria-label="tr('已保存的远程 Worker', 'Saved remote Workers')">
      <div class="worker-list-heading"><h3>{{ tr("远程节点", "Remote nodes") }}</h3><span>{{ tr(`${nodes.length} 个节点`, `${nodes.length} nodes`) }}</span></div>
      <article v-for="node in nodes" :key="node.id" class="panel worker-node">
        <header>
          <div class="node-title">
            <span class="node-icon"><Server :size="18" /></span>
            <div><strong>{{ node.name }}</strong><code>{{ node.endpoint }}</code></div>
          </div>
          <div class="node-actions">
            <button class="button ghost compact" type="button" :disabled="busyNode === node.id" @click="inspectNode(node)">
              <Activity :size="14" />{{ tr("检测", "Check") }}
            </button>
            <button class="button ghost compact" type="button" :disabled="busyNode === node.id" @click="syncNode(node)">
              <RefreshCw :size="14" :class="{ spinning: busyNode === node.id }" />{{ tr("同步结果", "Sync results") }}
            </button>
            <button class="icon-button danger" type="button" :title="tr('移除节点', 'Remove node')" :aria-label="tr(`移除节点 ${node.name}`, `Remove node ${node.name}`)" :disabled="busyNode === node.id" @click="removeNode(node)"><Trash2 :size="14" /></button>
          </div>
        </header>
        <p v-if="node.lastError" class="node-error" role="alert"><CircleAlert :size="14" /><span>{{ node.lastError }}</span></p>
        <p v-else-if="node.lastSeenAt" class="node-ok"><CheckCircle2 :size="14" /><span>{{ tr(`最后连接：${node.lastSeenAt}`, `Last connected: ${node.lastSeenAt}`) }}<template v-if="node.lastSyncAt"> · {{ tr(`最后同步：${node.lastSyncAt}`, `Last synced: ${node.lastSyncAt}`) }}</template></span></p>
        <div v-if="health[node.id]" class="node-health">
          <span>{{ health[node.id].os }} · {{ health[node.id].arch }}</span>
          <span>{{ tr(`运行 ${health[node.id].runningScans}`, `Running ${health[node.id].runningScans}`) }}</span>
          <span>{{ tr(`完成 ${health[node.id].completedScans}`, `Completed ${health[node.id].completedScans}`) }}</span>
          <span>{{ health[node.id].hostname }}</span>
        </div>
        <div v-if="environments[node.id]" class="remote-environment">
          <b>{{ tr("环境检测", "Environment check") }}</b>
          <span v-for="dep in environments[node.id].dependencies" :key="dep.name" :class="{ missing: !dep.available }">
            {{ dep.name }} · {{ dep.available ? dep.version || "OK" : tr(`缺失：${dep.detail}`, `Missing: ${dep.detail}`) }}
          </span>
          <span v-if="dependencyStatus(environments[node.id], 'docker')" :class="{ missing: !dependencyStatus(environments[node.id], 'docker')?.available }">
            Docker · {{ dependencyStatus(environments[node.id], 'docker')?.available ? dependencyStatus(environments[node.id], 'docker')?.version : dependencyStatus(environments[node.id], 'docker')?.detail }}
          </span>
        </div>
        <div v-if="scans[node.id]?.length" class="remote-scans">
          <div v-for="scan in scans[node.id].slice(0, 8)" :key="scan.id">
            <span><b>{{ scan.taskName || scan.projectName }}</b><small>{{ scan.status }} · {{ scan.currentCheckpoint }}</small></span>
            <span class="scan-actions">
              <button v-if="['queued','scanning'].includes(scan.status)" type="button" :title="tr('暂停', 'Pause')" :aria-label="tr('暂停任务', 'Pause task')" @click="control(node, scan, 'pause')"><Pause :size="12" /></button>
              <button v-if="['paused','failed','interrupted'].includes(scan.status)" type="button" :title="tr('继续', 'Resume')" :aria-label="tr('继续任务', 'Resume task')" @click="control(node, scan, 'resume')"><Play :size="12" /></button>
              <button v-if="!['completed','cancelled'].includes(scan.status)" type="button" :title="tr('取消', 'Cancel')" :aria-label="tr('取消任务', 'Cancel task')" @click="control(node, scan, 'cancel')"><Square :size="11" /></button>
            </span>
          </div>
        </div>
      </article>
      <div v-if="!loading && !nodes.length" class="panel worker-empty">{{ tr("尚未添加远程 Worker。先在 Intel Mac 或 Windows 上开启 Worker，再把地址和令牌填到上方。", "No remote Workers yet. Enable the Worker on an Intel Mac or Windows machine, then enter its address and token above.") }}</div>
    </section>

    <details class="panel worker-guide">
      <summary><span>{{ tr("连接与运行说明", "Connection and run guide") }}</span><small>{{ tr("Tailscale 私网 · 环境准备", "Tailscale private network · Environment setup") }}</small></summary>
      <div class="worker-guide-content">
        <ol>
          <li><b>{{ tr("两台电脑安装 Tailscale", "Install Tailscale on both computers") }}</b><span>{{ tr("使用同一个账号登录。无需公网 IP、路由器端口映射、OpenSSH 或反向代理。", "Sign in with the same account. No public IP, router port forwarding, OpenSSH, or reverse proxy is needed.") }}</span></li>
          <li><b>{{ tr("Intel Mac / Windows 运行对应平台的 Oviraptor", "Run the matching Oviraptor build on the Intel Mac / Windows machine") }}</b><span>{{ tr("进入本页完成环境检测，再开启本机 Worker，复制节点地址和令牌。", "Open this page, complete the environment check, enable the local Worker, and copy the node address and token.") }}</span></li>
          <li><b>{{ tr("M1 添加远程节点", "Add the remote node on the M1") }}</b><span>{{ tr("粘贴地址和令牌后点“检测”；任务在 Worker 上继续运行，主控端可查看、控制并同步结果。", "Paste the address and token, then click “Check”. Tasks keep running on the Worker; the controller can view, control, and sync results.") }}</span></li>
        </ol>
        <details class="worker-environment-guide">
          <summary>{{ tr("远程 Worker 运行环境（源码分析器依赖单独准备）", "Remote Worker environment (source analyzer dependencies are prepared separately)") }}</summary>
          <div class="manual-grid">
            <article>
              <Laptop :size="18" />
              <div><strong>{{ tr("macOS（Apple / Intel）", "macOS (Apple / Intel)") }}</strong><p>{{ tr("远程连接需要安装并登录 Tailscale；浏览器与 AST 工具需要 Node.js：", "Remote connections need Tailscale installed and signed in; browser and AST tools need Node.js:") }}</p><code>brew install node</code><p>{{ tr("资产采集和 Native HTTP 不需要 Python、Redis 或 Docker；可选源码分析器仍需要 Docker 和已批准的固定摘要镜像。", "Asset collection and Native HTTP do not need Python, Redis, or Docker; optional source analyzers still need Docker and approved pinned-digest images.") }}</p></div>
            </article>
            <article>
              <Server :size="18" />
              <div><strong>Windows 11 x64</strong><p>{{ tr("自动安装仅准备 Tailscale 与 Node.js LTS。手动环境检查：", "Automatic setup installs only Tailscale and Node.js LTS. Manual environment check:") }}</p><code>node --version; tailscale status</code><p>{{ tr("仅源码分析器需要另行准备 Docker 和固定摘要镜像。Native 工具能力须单独审批与隔离；当前 Windows Native HTTP artifact 写入适配器尚不可用。", "Only source analyzers need Docker and pinned-digest images prepared separately. Native tool capabilities require separate approval and isolation; the Windows Native HTTP artifact write adapter is not available yet.") }}</p></div>
            </article>
          </div>
        </details>
        <p class="worker-login-note">{{ tr("无人值守提示：两端必须保持开机、已登录系统且 Tailscale 在线。普通 macOS Tailscale 在重启后、尚未登录用户前通常不会联网。", "Unattended use: both machines must stay powered on, signed in, and online in Tailscale. The standard macOS Tailscale app usually does not connect after a restart until a user signs in.") }}</p>
      </div>
    </details>
  </div>
</template>

<style scoped>
.worker-settings{display:flex;flex-direction:column;gap:20px;min-width:0;container:worker-settings / inline-size;color:var(--ui-ink);font-size:var(--text-body)}
.worker-settings .panel{min-width:0;margin:0;padding:16px;border:1px solid var(--ui-line);border-radius:var(--radius-lg);background:var(--ui-surface);box-shadow:none}
.worker-section-heading{display:flex;justify-content:space-between;align-items:flex-start;gap:16px;margin-bottom:16px}
.worker-section-heading>div{min-width:0}
.worker-section-heading h3,.worker-list-heading h3{margin:0;font-size:var(--text-body);line-height:1.5;font-weight:600}
.worker-section-heading p{margin:5px 0 0;color:var(--ui-muted);font-size:var(--text-meta);line-height:1.6}
.worker-state{flex-shrink:0;padding:3px 8px;border-radius:var(--radius-pill);background:var(--ui-surface-sunken);color:var(--ui-muted);font-size:var(--text-micro);font-weight:600;line-height:1.6}
.worker-state.online{background:color-mix(in srgb,var(--green) 10%,var(--ui-surface));color:color-mix(in srgb,var(--green) 82%,var(--ui-ink))}
.worker-form{display:grid;gap:12px;align-items:end}
.local-form{grid-template-columns:minmax(0,1fr) 140px auto}
.node-form{grid-template-columns:minmax(140px,1fr) minmax(180px,1.25fr) minmax(180px,1.35fr) auto}
.worker-form label{min-width:0;display:flex;flex-direction:column;gap:6px}
.worker-form label>span{font-size:var(--text-meta);color:var(--ui-muted);line-height:1.4}
.worker-form input{box-sizing:border-box;width:100%;height:38px;min-width:0;padding:0 10px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface);color:var(--ui-ink);font:var(--text-body)/1.5 var(--font-sans);outline:0}
.worker-form input:focus{border-color:var(--ui-accent);box-shadow:0 0 0 3px color-mix(in srgb,var(--ui-accent) 14%,transparent)}
.worker-settings .button{display:inline-flex;align-items:center;justify-content:center;gap:6px;min-height:36px;height:36px;padding:0 12px;font-size:var(--text-meta);white-space:nowrap}
.worker-settings .icon-button{display:inline-flex;align-items:center;justify-content:center;flex-shrink:0;width:36px;min-width:36px;height:36px;padding:0;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface);color:var(--ui-muted)}
.worker-settings .icon-button.danger:hover:not(:disabled){color:var(--red);background:color-mix(in srgb,var(--red) 7%,var(--ui-surface));border-color:color-mix(in srgb,var(--red) 28%,var(--ui-line))}
.worker-settings button:focus-visible,.worker-settings summary:focus-visible{outline:2px solid var(--ui-accent);outline-offset:3px}
.worker-form .worker-toggle{position:relative;display:flex;flex-direction:row;align-items:center;gap:10px;height:38px;cursor:pointer}
.worker-toggle input{position:absolute;opacity:0;width:36px;height:22px;cursor:pointer}
.worker-toggle>span{flex-shrink:0;width:36px;height:22px;border-radius:var(--radius-pill);background:var(--ui-line-strong);position:relative}
.worker-toggle>span:after{content:"";position:absolute;left:3px;top:3px;width:16px;height:16px;border-radius:50%;background:#fff;transition:transform .18s;box-shadow:0 1px 3px #0002}
.worker-toggle input:checked+span{background:var(--ui-accent)}
.worker-toggle input:checked+span:after{transform:translateX(14px)}
.worker-toggle input:focus-visible+span{outline:2px solid var(--ui-accent);outline-offset:3px}
.worker-toggle b{font-size:var(--text-meta);font-weight:500}
.worker-status{margin:12px 0;padding:8px 10px;border-radius:var(--radius-md);background:var(--ui-surface-sunken);color:var(--ui-muted);font-size:var(--text-meta);line-height:1.6;overflow-wrap:anywhere}
.worker-status.error{background:color-mix(in srgb,var(--amber) 8%,var(--ui-surface));color:color-mix(in srgb,var(--amber) 82%,var(--ui-ink))}
.connection-values{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px}
.connection-value{min-width:0;display:grid;grid-template-columns:minmax(0,1fr) auto;grid-template-rows:auto 1fr;column-gap:10px;row-gap:5px;padding:10px 12px;border:1px solid var(--ui-line);border-radius:var(--radius-md)}
.connection-value>span{font-size:var(--text-micro);color:var(--ui-muted);line-height:1.4}
.connection-value code{grid-column:1;min-width:0;align-self:center;overflow-wrap:anywhere;white-space:normal;font-size:var(--text-meta);line-height:1.6;user-select:text}
.connection-value .icon-button{grid-column:2;grid-row:1/3;align-self:center;width:30px;min-width:30px;height:30px;border-color:transparent}
.worker-lifecycle-note{margin:10px 0 0;font-size:var(--text-micro);color:var(--ui-muted);line-height:1.6}
.worker-node-list{display:flex;flex-direction:column;gap:10px;min-width:0}
.worker-list-heading{display:flex;justify-content:space-between;align-items:center;gap:12px;margin-bottom:2px}
.worker-list-heading>span{font-size:var(--text-meta);color:var(--ui-muted)}
.worker-node>header{display:flex;align-items:center;justify-content:space-between;gap:16px}
.node-title{display:flex;min-width:0;align-items:center;gap:10px}
.node-icon{flex-shrink:0;width:34px;height:34px;border-radius:var(--radius-md);background:color-mix(in srgb,var(--ui-accent) 8%,var(--ui-surface));color:var(--ui-accent);display:grid;place-items:center}
.node-title>div{min-width:0;display:flex;flex-direction:column;gap:3px}
.node-title strong{font-size:var(--text-body);line-height:1.4;overflow-wrap:anywhere}
.node-title code{color:var(--ui-muted);font-size:var(--text-meta);line-height:1.5;overflow-wrap:anywhere;white-space:normal}
.node-actions{display:flex;flex-shrink:0;align-items:center;gap:8px}
.node-ok,.node-error{display:flex;align-items:flex-start;gap:7px;margin:12px 0 0;padding:8px 10px;border-radius:var(--radius-md);font-size:var(--text-meta);line-height:1.6;overflow-wrap:anywhere}
.node-ok svg,.node-error svg{flex-shrink:0;margin-top:3px}
.node-ok{color:var(--ui-muted);background:var(--ui-surface-sunken)}
.node-error{color:color-mix(in srgb,var(--red) 85%,var(--ui-ink));background:color-mix(in srgb,var(--red) 6%,var(--ui-surface))}
.node-health{display:flex;gap:6px;flex-wrap:wrap;margin-top:12px}
.node-health span{padding:3px 8px;border-radius:var(--radius-sm);background:var(--ui-surface-sunken);color:var(--ui-muted);font-size:var(--text-micro);line-height:1.6;overflow-wrap:anywhere}
.remote-environment{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:8px;margin-top:12px;padding-top:12px;border-top:1px solid var(--ui-line)}
.remote-environment b{grid-column:1/-1;font-size:var(--text-meta)}
.remote-environment span{padding:8px;border-radius:var(--radius-sm);background:var(--ui-surface-sunken);color:color-mix(in srgb,var(--green) 82%,var(--ui-ink));font-size:var(--text-micro);line-height:1.6;overflow-wrap:anywhere}
.remote-environment span.missing{background:color-mix(in srgb,var(--red) 8%,var(--ui-surface));color:color-mix(in srgb,var(--red) 82%,var(--ui-ink))}
.remote-scans{margin-top:12px;border-top:1px solid var(--ui-line)}
.remote-scans>div{display:flex;align-items:center;justify-content:space-between;gap:10px;padding:10px 0;border-bottom:1px solid var(--ui-line)}
.remote-scans>div:last-child{border-bottom:0;padding-bottom:0}
.remote-scans>div>span:first-child{display:flex;min-width:0;flex-direction:column;gap:3px;overflow-wrap:anywhere}
.remote-scans b{font-size:var(--text-meta)}
.remote-scans small{font-size:var(--text-micro);color:var(--ui-muted);line-height:1.5}
.scan-actions{display:flex;flex-shrink:0;gap:6px}
.scan-actions button{display:grid;place-items:center;width:30px;height:30px;padding:0;border:1px solid var(--ui-line);border-radius:var(--radius-sm);background:var(--ui-surface);color:var(--ui-muted)}
.scan-actions button:hover{background:var(--ui-surface-raised);color:var(--ui-ink)}
.worker-settings .worker-empty{padding:24px 16px;text-align:center;color:var(--ui-muted);font-size:var(--text-meta);line-height:1.6}
.worker-guide>summary{color:var(--ui-ink);font-size:var(--text-meta);line-height:1.6}
.worker-guide>summary>span{font-weight:500}
.worker-guide>summary>small{margin-left:12px;color:var(--ui-muted);font-size:var(--text-micro)}
.worker-guide-content{margin-top:16px;padding-top:16px;border-top:1px solid var(--ui-line)}
.worker-guide ol{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:16px;margin:0;padding:0;list-style:none;counter-reset:guide}
.worker-guide li{counter-increment:guide;display:grid;grid-template-columns:22px minmax(0,1fr);align-content:start;gap:5px 8px;min-width:0}
.worker-guide li:before{content:counter(guide);grid-row:1/3;width:22px;height:22px;border-radius:50%;display:grid;place-items:center;background:var(--ui-surface-sunken);color:var(--ui-muted);font-size:var(--text-micro);font-weight:600}
.worker-guide li b{font-size:var(--text-meta);line-height:1.5}
.worker-guide li span{grid-column:2;font-size:var(--text-micro);line-height:1.7;color:var(--ui-muted)}
.worker-environment-guide{margin-top:16px;padding-top:12px;border-top:1px solid var(--ui-line)}
.worker-environment-guide>summary{color:var(--ui-muted);font-size:var(--text-meta);line-height:1.6}
.manual-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px;margin-top:12px}
.manual-grid article{display:flex;align-items:flex-start;gap:10px;min-width:0;padding:12px;border:1px solid var(--ui-line);border-radius:var(--radius-md)}
.manual-grid article>svg{flex-shrink:0;color:var(--ui-muted)}
.manual-grid article>div{display:flex;min-width:0;flex-direction:column;gap:6px}
.manual-grid strong{font-size:var(--text-meta)}
.manual-grid p{margin:0;color:var(--ui-muted);font-size:var(--text-micro);line-height:1.7}
.manual-grid code{padding:8px;border-radius:var(--radius-sm);background:var(--ui-surface-sunken);color:var(--ui-ink);white-space:pre-wrap;overflow-wrap:anywhere;font-size:var(--text-micro);line-height:1.6}
.worker-login-note{margin:12px 0 0;color:var(--ui-muted);font-size:var(--text-micro);line-height:1.7}
@container worker-settings (max-width:850px){.node-form{grid-template-columns:repeat(2,minmax(0,1fr))}.node-form .button{justify-self:start;align-self:end}.worker-guide ol{grid-template-columns:1fr}.remote-environment{grid-template-columns:repeat(2,minmax(0,1fr))}}
@container worker-settings (max-width:580px){.local-form{grid-template-columns:minmax(0,1fr) 120px}.local-form .button{grid-column:1/-1;justify-self:start}.connection-values{grid-template-columns:1fr}.worker-node>header{align-items:flex-start;flex-direction:column;gap:12px}.node-actions{flex-wrap:wrap}.manual-grid{grid-template-columns:1fr}.worker-guide>summary>small{display:block;margin:4px 0 0 15px}}
@container worker-settings (max-width:400px){.worker-settings .panel{padding:14px}.node-form,.local-form,.remote-environment{grid-template-columns:1fr}.local-form .button{grid-column:auto}.node-actions{gap:6px}.node-actions .button{padding:0 10px}.worker-section-heading{gap:10px}.worker-state{padding-inline:6px}}
</style>
