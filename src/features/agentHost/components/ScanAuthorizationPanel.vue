<script setup lang="ts">
// Operator-only authorization panel rendered in the chat when the agent's
// `request_scan_authorization` tool raises a `scan.authorization.requested`
// event. The event is purely advisory — it carries no authority and never
// touches the authorization table. Only this panel's own "Authorize" click,
// routed through the host's operator-only `scan-authorization` conversation
// action, can land a capability-class grant, and only after the operator
// ticks the legal-authorization attestation themselves. The agent cannot
// drive this panel or pre-tick the attestation on its behalf — see
// scanAuthorizationConversationAction in packages/agent-host/src/scanning-tasks.mjs.
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { ShieldAlert } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { AgentHostChatClient } from "../chatClient";
import type { DiscoveryWitness } from "../discoveryTypes";

export type PendingScanAuthorizationRequest = { conversationId: string; taskId: string; proposedClasses: string[]; reason: string };

const props = defineProps<{ client: AgentHostChatClient; conversationId: string; projectId: number; request: PendingScanAuthorizationRequest }>();
const emit = defineEmits<{ resolved: [taskId: string] }>();
const { tr } = useI18n();

// Canonical high-risk capability classes — mirror of
// scan_capability_authorization.rs::HIGH_RISK_CLASS_STRINGS,
// AgentWorkbench.vue::HIGH_RISK_CLASSES and the agent-host's
// SCAN_AUTH_LEGAL_CLASSES. Keep all four in sync.
const HIGH_RISK_CLASSES = ["internal_scope", "target_write", "code_exec", "egress", "post_write", "persistence", "exfil", "waf_evasion"] as const;
function classLabel(cls: string): string {
  switch (cls) {
    case "code_exec": return tr("命令执行 / RCE", "Command execution / RCE");
    case "target_write": return tr("对目标写入或改动", "Write / modify the target");
    case "post_write": return tr("POST / 上传", "POST / upload");
    case "egress": return tr("出网与 OOB 外带（DNSlog 等）", "Egress & OOB callback (DNSlog etc.)");
    case "internal_scope": return tr("内网 / SSRF 深入", "Internal / SSRF reach");
    case "persistence": return tr("在目标上建立持久化", "Establish persistence on target");
    case "exfil": return tr("数据外带", "Data exfiltration");
    case "waf_evasion": return tr("WAF 智能绕过", "Intelligent WAF evasion");
    default: return cls;
  }
}
function classHint(cls: string): string {
  switch (cls) {
    case "code_exec": return tr("仅无害命令（id/whoami/ifconfig）可自动执行；其他命令逐次人工批准。", "Only harmless commands (id/whoami/ifconfig) auto-run; anything else needs per-action approval.");
    case "target_write": return tr("变更类探测需人工批准。", "Mutating probes need approval.");
    case "post_write": return tr("仍须在任务里当场确认每一次写请求；不确认不会发包。", "Each write still needs a live confirmation on the task; nothing is sent without it.");
    case "egress": return tr("盲注回连，需要自己的 DNSLog 或 httptool 地址。", "Blind callback; needs your own DNSLog or httptool URL.");
    case "internal_scope": return tr("只跟进响应中出现的内网地址，不预先指定，也不扫网段。", "Follows only an internal address seen in a response; nothing preselected, no range scan.");
    case "persistence": return tr("高度不可逆，默认证明即止、可回滚。", "Highly irreversible; proof-only and reversible by default.");
    case "exfil": return tr("仅取证明影响所需的最小样本。", "Only the minimal sample needed to prove impact.");
    case "waf_evasion": return tr("允许对拦截进行多次智能绕过尝试。", "Allows multiple intelligent evasion attempts against blocking.");
    default: return "";
  }
}

const witness = ref<DiscoveryWitness>();
const legalClasses = ref<string[]>([...HIGH_RISK_CLASSES]);
const loading = ref(true);
const busy = ref(false);
const error = ref("");
const notice = ref("");
const resolved = ref(false);
const legalAttestation = ref(false);
// Final say is the operator's: pre-select the model's proposal as a starting
// point only, never auto-authorized.
const classes = ref<string[]>([...new Set(props.request.proposedClasses)].filter(c => (HIGH_RISK_CLASSES as readonly string[]).includes(c)));
let disposed = false;

const ossDeepAnalysis = computed<boolean>({
  get: () => classes.value.includes("target_write"),
  set: (on: boolean) => { toggleClass("target_write", on); },
});
function toggleClass(cls: string, on?: boolean) {
  const has = classes.value.includes(cls);
  const want = on === undefined ? !has : on;
  if (want === has) return;
  classes.value = want ? [...classes.value, cls] : classes.value.filter(c => c !== cls);
}
const canAuthorize = computed(() => !loading.value && !busy.value && !resolved.value && Boolean(witness.value) && (classes.value.length === 0 || legalAttestation.value));

async function load() {
  loading.value = true; error.value = "";
  try {
    const status = await props.client.scanAuthorizationStatus(props.conversationId, props.projectId);
    if (disposed) return;
    witness.value = status.confirmationWitness;
    if (Array.isArray(status.legalClasses) && status.legalClasses.length) legalClasses.value = status.legalClasses;
  } catch (e) {
    if (!disposed) error.value = String((e as Error).message || e);
  } finally { if (!disposed) loading.value = false; }
}

function describeError(code: string): string {
  if (code === "host_scan_authorization_stale") return tr("授权状态已变化，已重新读取；请核对后再次提交。", "The authorization state changed. Reloaded — please review and submit again.");
  if (code === "host_scan_authorization_attestation_required") return tr("勾选的能力类非空，必须先勾选合法授权确认。", "A non-empty class selection requires the legal-authorization checkbox first.");
  if (code === "host_tool_permission_denied" || code === "host_tool_authority_changed") return tr("当前 Agent 接收方没有执行权限，或权限已变化；请到「数据权限」重新确认。", "The current Agent recipient lacks execute permission, or authority changed. Reconfirm under Data permissions.");
  if (code === "host_scan_executor_unavailable") return tr("扫描执行器暂不可用。", "The scan executor is unavailable.");
  return code;
}

async function authorize(decline = false) {
  if (busy.value || loading.value || resolved.value || !witness.value) return;
  const submitClasses = decline ? [] : [...classes.value];
  const submitAttestation = decline ? false : legalAttestation.value;
  if (submitClasses.length > 0 && !submitAttestation) { error.value = describeError("host_scan_authorization_attestation_required"); return; }
  busy.value = true; error.value = ""; notice.value = "";
  try {
    await props.client.authorizeScanCapability(props.conversationId, props.projectId, {
      ...witness.value, taskId: props.request.taskId, classes: submitClasses, legalAttestation: submitAttestation,
    });
    if (disposed) return;
    resolved.value = true;
    notice.value = submitClasses.length
      ? tr(`已授权 ${submitClasses.length} 项高危能力类；每次具体高危动作仍需人工逐次批准。`, `Authorized ${submitClasses.length} high-risk class(es); each concrete high-risk action still needs per-action approval.`)
      : tr("已记录：不授权任何高危能力类。", "Recorded: no high-risk classes authorized.");
    setTimeout(() => { if (!disposed) emit("resolved", props.request.taskId); }, 1400);
  } catch (e) {
    if (disposed) return;
    const code = String((e as Error).message || e);
    error.value = describeError(code);
    if (code === "host_scan_authorization_stale" || code === "host_tool_authority_changed") await load();
  } finally { if (!disposed) busy.value = false; }
}

onMounted(load);
onBeforeUnmount(() => { disposed = true; });
</script>

<template>
  <section class="scan-auth-panel" data-testid="scan-authorization-panel" role="group">
    <header>
      <ShieldAlert :size="16" aria-hidden="true" />
      <div>
        <strong>{{ tr("扫描能力授权请求", "Scan capability authorization request") }}</strong>
        <small>{{ tr("Agent 仅提议，授权永远由你点选决定。", "The agent only proposes — you decide and click to authorize.") }}</small>
      </div>
    </header>
    <p v-if="request.reason" class="scan-auth-reason">{{ request.reason }}</p>
    <p v-if="loading" role="status">{{ tr("正在读取当前授权状态…", "Reading the current authorization state…") }}</p>
    <template v-else-if="!resolved">
      <ul class="scan-auth-classes">
        <li v-for="cls in HIGH_RISK_CLASSES" :key="cls" :class="{ proposed: request.proposedClasses.includes(cls) }">
          <label>
            <input type="checkbox" :checked="classes.includes(cls)" :disabled="busy" data-testid="scan-auth-class" @change="toggleClass(cls, ($event.target as HTMLInputElement).checked)" />
            <span><strong>{{ classLabel(cls) }}</strong><small>{{ classHint(cls) }}</small></span>
          </label>
          <small v-if="request.proposedClasses.includes(cls)" class="scan-auth-proposed-tag">{{ tr("Agent 建议", "Agent-suggested") }}</small>
        </li>
      </ul>
      <label class="scan-auth-oss">
        <input type="checkbox" v-model="ossDeepAnalysis" :disabled="busy" data-testid="scan-auth-oss-toggle" />
        <span>{{ tr("开源框架深度分析（等同勾选“对目标写入或改动”）", "Open-source framework deep analysis (equivalent to ticking “write / modify the target”)") }}</span>
      </label>
      <label class="scan-auth-attest" :class="{ 'attest-missing': classes.length > 0 && !legalAttestation }">
        <input type="checkbox" v-model="legalAttestation" :disabled="busy" data-testid="scan-auth-attestation" />
        {{ tr("我确认已获得对以上目标的合法书面授权，并对使用这些能力负责。", "I confirm I hold written legal authorization for these targets and take responsibility for using these capabilities.") }}
      </label>
      <small v-if="classes.length > 0 && !legalAttestation" class="scan-auth-warn">{{ tr("已勾选能力类：必须先勾选合法授权确认才能授权。", "Classes selected: the legal-authorization checkbox is required before you can authorize.") }}</small>
      <p v-if="error" class="scan-auth-error" role="alert">{{ error }}</p>
      <div class="scan-auth-actions">
        <button type="button" class="button ghost compact" :disabled="busy || loading" data-testid="scan-auth-decline" @click="authorize(true)">{{ busy ? tr("提交中…", "Submitting…") : tr("不授权任何能力", "Authorize none") }}</button>
        <button type="button" class="button primary compact" :disabled="!canAuthorize" data-testid="scan-auth-submit" @click="authorize(false)">{{ busy ? tr("正在授权…", "Authorizing…") : tr(`授权所选 ${classes.length} 项`, `Authorize ${classes.length} selected`) }}</button>
      </div>
    </template>
    <p v-if="notice" class="scan-auth-notice" role="status">{{ notice }}</p>
  </section>
</template>

<style scoped>
.scan-auth-panel{border:1px solid color-mix(in srgb,var(--ui-accent,#2878ff) 35%,var(--ui-line));border-radius:var(--radius-md);padding:14px;margin:0 0 12px;background:color-mix(in srgb,var(--ui-accent,#2878ff) 4%,var(--ui-surface));font-size:var(--text-meta);color:var(--ui-ink)}
.scan-auth-panel header{display:flex;align-items:flex-start;gap:9px}
.scan-auth-panel header svg{flex-shrink:0;margin-top:2px;color:#95641e}
.scan-auth-panel header strong{display:block;font-size:var(--text-body);font-weight:600}
.scan-auth-panel header small{display:block;margin-top:2px;color:var(--ui-muted)}
.scan-auth-reason{margin:10px 0 0;padding:8px 10px;border:1px solid var(--ui-line);border-radius:var(--radius-sm);background:var(--ui-surface-raised);color:var(--ui-muted);overflow-wrap:anywhere}
.scan-auth-classes{list-style:none;margin:12px 0;padding:0;display:grid;gap:8px}
.scan-auth-classes li{display:flex;align-items:flex-start;justify-content:space-between;gap:10px;padding:8px 10px;border:1px solid var(--ui-line);border-radius:var(--radius-sm);background:var(--ui-surface)}
.scan-auth-classes li.proposed{border-color:color-mix(in srgb,var(--ui-accent,#2878ff) 45%,var(--ui-line))}
.scan-auth-classes label{display:flex;align-items:flex-start;gap:9px;cursor:pointer;min-width:0}
.scan-auth-classes label>span{min-width:0}
.scan-auth-classes label>span>strong{display:block;font-size:var(--text-meta)}
.scan-auth-classes label>span>small{display:block;margin-top:2px;color:var(--ui-muted);line-height:1.5}
.scan-auth-classes input{margin-top:2px;flex-shrink:0}
.scan-auth-proposed-tag{flex-shrink:0;color:color-mix(in srgb,var(--ui-accent,#2878ff) 80%,var(--ui-ink));white-space:nowrap}
.scan-auth-oss{display:flex;align-items:flex-start;gap:9px;margin:0 0 10px;padding:8px 10px;border:1px dashed var(--ui-line);border-radius:var(--radius-sm);cursor:pointer}
.scan-auth-attest{display:flex;align-items:flex-start;gap:9px;padding:9px 10px;border:1px solid var(--ui-line);border-radius:var(--radius-sm);background:var(--ui-surface-raised);cursor:pointer;line-height:1.6}
.scan-auth-attest.attest-missing{border-color:color-mix(in srgb,var(--red,#ac3939) 45%,var(--ui-line))}
.scan-auth-warn{display:block;margin-top:6px;color:#95641e}
.scan-auth-error{margin-top:8px;color:var(--red,#ac3939)}
.scan-auth-notice{margin-top:8px;color:#246c4e}
.scan-auth-actions{display:flex;justify-content:flex-end;gap:8px;margin-top:12px}
.scan-auth-panel input[type=checkbox]{flex-shrink:0;appearance:auto;width:14px;height:14px;accent-color:var(--ui-accent,#2878ff);cursor:pointer}
.scan-auth-panel input:disabled{cursor:default}
</style>
