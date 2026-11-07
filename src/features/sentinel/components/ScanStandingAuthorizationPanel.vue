<script setup lang="ts">
// Operator-only page/panel for registering a long-standing (per target-pattern)
// high-risk capability authorization, so a target the operator has already
// cleared does not need re-authorizing on every new scan task. This writes
// directly to the native `set_scan_standing_authorization` Tauri command — no
// agent tool reaches this command, and submitting here is always the
// operator's own explicit click, gated on the same legal-authorization
// attestation as the per-task panel (AgentWorkbench.vue / SentinelBoard.vue).
//
// There is currently no companion "list/get" command for standing
// authorizations, so this panel cannot read back what the native side has
// stored; it only shows what was registered in this session, for the
// operator's own reference.
import { computed, reactive, ref } from "vue";
import { ShieldPlus } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import { sentinelApi } from "../api";

const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();
const { tr } = useI18n();

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

type StandingEntry = { targetPattern: string; classes: string[]; budget?: number; expiresAt?: string; createdAt: string };

const form = reactive({
  targetPattern: "",
  classes: [] as string[],
  legalAttestation: false,
  budget: undefined as number | undefined,
  expiresAt: "", // datetime-local string, converted to ISO on submit
});
const busy = ref(false);
const error = ref("");
const registered = ref<StandingEntry[]>([]);

const ossDeepAnalysis = computed<boolean>({
  get: () => form.classes.includes("target_write"),
  set: (on: boolean) => toggleClass("target_write", on),
});
function toggleClass(cls: string, on?: boolean) {
  const has = form.classes.includes(cls);
  const want = on === undefined ? !has : on;
  if (want === has) return;
  form.classes = want ? [...form.classes, cls] : form.classes.filter(c => c !== cls);
}
const canSubmit = computed(() => !busy.value && form.targetPattern.trim().length > 0 && (form.classes.length === 0 || form.legalAttestation));

async function submit() {
  const targetPattern = form.targetPattern.trim();
  if (!targetPattern) { error.value = tr("请先填写目标模式（域名、通配符或 URL 前缀）。", "Enter a target pattern (domain, wildcard, or URL prefix) first."); return; }
  if (form.classes.length > 0 && !form.legalAttestation) { error.value = tr("已勾选能力类：必须先勾选合法授权确认。", "Classes selected: the legal-authorization checkbox is required first."); return; }
  let expiresAt: string | undefined;
  if (form.expiresAt) {
    const parsed = new Date(form.expiresAt);
    if (Number.isNaN(parsed.getTime())) { error.value = tr("到期时间格式无效。", "Invalid expiry date/time."); return; }
    expiresAt = parsed.toISOString();
  }
  busy.value = true; error.value = "";
  try {
    await sentinelApi.setScanStandingAuthorization({
      targetPattern,
      classes: [...form.classes],
      legalAttestation: form.legalAttestation,
      budget: form.budget,
      expiresAt,
    });
    registered.value = [{ targetPattern, classes: [...form.classes], budget: form.budget, expiresAt, createdAt: new Date().toISOString() }, ...registered.value];
    emit("notify", "success", tr("长期授权已登记。", "Standing authorization registered."));
    form.targetPattern = ""; form.classes = []; form.legalAttestation = false; form.budget = undefined; form.expiresAt = "";
  } catch (e) {
    error.value = String((e as Error).message || e);
    emit("notify", "error", `${tr("长期授权登记失败", "Standing authorization failed")}: ${error.value}`);
  } finally { busy.value = false; }
}
</script>

<template>
  <section class="standing-auth" data-testid="scan-standing-authorization-panel">
    <header class="standing-auth-heading">
      <h3><ShieldPlus :size="17" />{{ tr("长期扫描授权", "Standing scan authorization") }}</h3>
      <p>{{ tr("为已获得长期合法授权的目标登记高危能力类，避免每次新扫描任务都重新授权；仍需在此一次性勾选合法授权确认。", "Register high-risk classes for a target you already hold long-term legal authorization for, so new scan tasks against it need not be re-authorized each time. Still requires ticking the legal-authorization confirmation once here.") }}</p>
    </header>
    <div class="standing-auth-form">
      <label class="field">
        <span>{{ tr("目标模式（域名 / 通配符 / URL 前缀）", "Target pattern (domain / wildcard / URL prefix)") }}</span>
        <input v-model.trim="form.targetPattern" type="text" :disabled="busy" :placeholder="tr('例如 *.example.com 或 https://app.example.com', 'e.g. *.example.com or https://app.example.com')" data-testid="standing-auth-pattern" />
      </label>
      <div class="field-row">
        <label class="field">
          <span>{{ tr("预算上限（可空）", "Budget cap (optional)") }}</span>
          <input v-model.number="form.budget" type="number" min="0" step="1" :disabled="busy" data-testid="standing-auth-budget" />
        </label>
        <label class="field">
          <span>{{ tr("到期时间（可空）", "Expiry (optional)") }}</span>
          <input v-model="form.expiresAt" type="datetime-local" :disabled="busy" data-testid="standing-auth-expires" />
        </label>
      </div>
      <ul class="standing-auth-classes">
        <li v-for="cls in HIGH_RISK_CLASSES" :key="cls">
          <label>
            <input type="checkbox" :checked="form.classes.includes(cls)" :disabled="busy" data-testid="standing-auth-class" @change="toggleClass(cls, ($event.target as HTMLInputElement).checked)" />
            {{ classLabel(cls) }}
          </label>
        </li>
      </ul>
      <label class="standing-auth-oss">
        <input type="checkbox" v-model="ossDeepAnalysis" :disabled="busy" data-testid="standing-auth-oss-toggle" />
        <span>{{ tr("开源框架深度分析（等同勾选“对目标写入或改动”）", "Open-source framework deep analysis (equivalent to ticking “write / modify the target”)") }}</span>
      </label>
      <label class="standing-auth-attest" :class="{ 'attest-missing': form.classes.length > 0 && !form.legalAttestation }">
        <input type="checkbox" v-model="form.legalAttestation" :disabled="busy" data-testid="standing-auth-attestation" />
        {{ tr("我确认已获得对以上目标模式的合法书面授权，并对使用这些能力负责。", "I confirm I hold written legal authorization for this target pattern and take responsibility for using these capabilities.") }}
      </label>
      <p v-if="error" class="standing-auth-error" role="alert">{{ error }}</p>
      <div class="standing-auth-actions">
        <button type="button" class="button primary compact" :disabled="!canSubmit" data-testid="standing-auth-submit" @click="submit">{{ busy ? tr("登记中…", "Registering…") : tr("登记长期授权", "Register standing authorization") }}</button>
      </div>
    </div>
    <div v-if="registered.length" class="standing-auth-registered">
      <strong>{{ tr("本次会话已登记", "Registered this session") }}</strong>
      <ul>
        <li v-for="(entry, index) in registered" :key="index">
          <code>{{ entry.targetPattern }}</code>
          <span>{{ entry.classes.length ? entry.classes.map(classLabel).join(tr('、', ', ')) : tr('无能力类', 'no classes') }}</span>
          <small v-if="entry.budget">{{ tr(`预算 ${entry.budget}`, `budget ${entry.budget}`) }}</small>
          <small v-if="entry.expiresAt">{{ tr(`至 ${entry.expiresAt}`, `until ${entry.expiresAt}`) }}</small>
        </li>
      </ul>
      <small class="standing-auth-registered-note">{{ tr("此列表仅为本次会话的提交记录；没有读取接口可核对原生侧的实际存储状态。", "This list only reflects submissions made in this session; there is no read-back endpoint to verify the native side's stored state.") }}</small>
    </div>
  </section>
</template>

<style scoped>
.standing-auth{container-type:inline-size;min-width:0;margin-top:20px;padding-top:20px;border-top:1px solid var(--ui-line);color:var(--ui-ink);font:var(--text-body)/1.6 var(--font-sans)}
.standing-auth-heading h3{display:flex;align-items:center;gap:8px;margin:0;font-size:var(--text-heading);font-weight:600}
.standing-auth-heading h3 svg{color:var(--ui-muted)}
.standing-auth-heading p{margin:4px 0 16px;color:var(--ui-muted);font-size:var(--text-meta)}
.standing-auth-form{display:grid;gap:12px;max-width:560px}
.standing-auth-form .field{display:grid;gap:6px}
.standing-auth-form .field>span{color:var(--ui-muted);font-size:var(--text-meta);font-weight:600}
.standing-auth-form input[type=text],.standing-auth-form input[type=number],.standing-auth-form input[type=datetime-local]{height:36px;padding:0 10px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface);color:var(--ui-ink);font:var(--text-body)/1.5 var(--font-sans)}
.standing-auth-form .field-row{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px}
.standing-auth-classes{list-style:none;margin:4px 0;padding:0;display:grid;gap:6px}
.standing-auth-classes label{display:flex;align-items:center;gap:8px;cursor:pointer;font-size:var(--text-meta)}
.standing-auth-oss{display:flex;align-items:flex-start;gap:9px;padding:8px 10px;border:1px dashed var(--ui-line);border-radius:var(--radius-sm);cursor:pointer;font-size:var(--text-meta)}
.standing-auth-attest{display:flex;align-items:flex-start;gap:9px;padding:9px 10px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised);cursor:pointer;line-height:1.6;font-size:var(--text-meta)}
.standing-auth-attest.attest-missing{border-color:color-mix(in srgb,var(--red,#ac3939) 45%,var(--ui-line))}
.standing-auth-error{margin:0;color:var(--red,#ac3939)}
.standing-auth-actions{display:flex;justify-content:flex-end}
.standing-auth-registered{margin-top:18px;padding-top:14px;border-top:1px solid var(--ui-line);font-size:var(--text-meta)}
.standing-auth-registered ul{list-style:none;margin:8px 0;padding:0;display:grid;gap:6px}
.standing-auth-registered li{display:flex;flex-wrap:wrap;align-items:center;gap:8px;padding:6px 0}
.standing-auth-registered code{font:var(--text-meta)/1.5 var(--font-mono)}
.standing-auth-registered small{color:var(--ui-muted)}
.standing-auth-registered-note{display:block;margin-top:6px;color:var(--ui-muted)}
.standing-auth input[type=checkbox]{flex-shrink:0;appearance:auto;width:14px;height:14px;accent-color:var(--ui-accent,#2878ff);cursor:pointer}
.standing-auth input:disabled{cursor:default}
@container(max-width:600px){.standing-auth-form .field-row{grid-template-columns:minmax(0,1fr)}}
</style>
