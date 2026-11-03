<script setup lang="ts">
import { ref } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";

// Operator entry points for the capability commands that run off the agent budget
// and attribute their findings to this scan: decompiled-artifact analysis (#1),
// exposed-endpoint probing (#4), cloud-bucket exposure (#5) and JWT weakness
// analysis (#8). The dangerous tiers (bucket write canary) additionally require the
// scan to have authorized the matching high-risk class; the backend enforces that
// and returns a clear error otherwise. Collapsed by default so it stays out of the way.
const props = defineProps<{ scanId: string }>();
const { tr } = useI18n();

const artifactDir = ref("");
const exposureUrl = ref("");
const bucketUrl = ref("");
const bucketCanary = ref(false);
const jwtToken = ref("");
const jwtSecrets = ref("");
const sweepUrl = ref("");

const busy = ref("");
const result = ref<Record<string, string>>({});
const errorText = ref<Record<string, string>>({});

async function run(tool: string, fn: () => Promise<string>) {
  if (busy.value) return;
  busy.value = tool;
  errorText.value = { ...errorText.value, [tool]: "" };
  result.value = { ...result.value, [tool]: "" };
  try {
    result.value = { ...result.value, [tool]: await fn() };
  } catch (e) {
    errorText.value = { ...errorText.value, [tool]: String(e) };
  } finally {
    busy.value = "";
  }
}

function runArtifact() {
  return run("artifact", async () => {
    const n = await sentinelApi.analyzeDecompiledArtifact(props.scanId, artifactDir.value.trim());
    return tr(`已写入 ${n} 条发现`, `Wrote ${n} findings`);
  });
}
function runExposure() {
  return run("exposure", async () => {
    const n = await sentinelApi.probeExposedEndpoints(props.scanId, exposureUrl.value.trim());
    return tr(`命中 ${n} 个暴露端点`, `${n} exposed endpoints`);
  });
}
function runBucket() {
  return run("bucket", async () => {
    const r = await sentinelApi.probeCloudBucket(props.scanId, bucketUrl.value.trim(), bucketCanary.value);
    const parts = [
      `${r.provider}`,
      r.publicListing ? tr("可匿名列举", "public listing") : tr("不可列举", "no listing"),
      tr(`发现 ${r.findings} 条`, `${r.findings} findings`),
    ];
    return parts.join(" · ");
  });
}
function runSweep() {
  return run("sweep", async () => {
    const r = await sentinelApi.runAttackSweep(props.scanId, sweepUrl.value.trim());
    const summary = `${tr("探测注入点", "probed points")} ${r.probed} · ${tr("可注入", "injectable")} ${r.injectable}`;
    if (r.injectable === 0 && (r.note || r.hint)) return `${summary} · ${r.note || r.hint}`;
    return summary;
  });
}
function runJwt() {
  return run("jwt", async () => {
    const secrets = jwtSecrets.value.split("\n").map((s) => s.trim()).filter(Boolean);
    const r = await sentinelApi.analyzeJwt(props.scanId, jwtToken.value.trim(), secrets);
    const bits = [`alg=${r.alg || "?"}`];
    if (r.weaknesses?.length) bits.push(r.weaknesses.join(","));
    if (r.crackedSecretIndex !== null && r.crackedSecretIndex !== undefined)
      bits.push(tr(`命中第 ${r.crackedSecretIndex + 1} 个候选密钥`, `secret #${r.crackedSecretIndex + 1} matched`));
    if (!r.weaknesses?.length) bits.push(tr("未发现可伪造弱点", "no forgeable weakness"));
    return bits.join(" · ");
  });
}
</script>

<template>
  <details v-if="props.scanId" class="manual-caps">
    <summary>{{ tr("手动能力（对本任务目标）", "Manual capabilities (for this task)") }}</summary>
    <small class="mc-note">{{ tr("这些是操作者手动触发的取证动作，结果计入本任务。危险层（桶写入 canary）需先在创建任务时授权对应高危能力。", "Operator-triggered forensic actions; results attribute to this task. The dangerous tier (bucket write canary) needs the matching high-risk class authorized at task creation.") }}</small>

    <div class="mc-tool">
      <label>{{ tr("① 反编译产物分析（解包目录）", "① Decompiled-artifact analysis (unpacked dir)") }}</label>
      <div class="mc-row">
        <input v-model="artifactDir" :placeholder="tr('/path/to/unpacked', '/path/to/unpacked')" />
        <button class="button primary" :disabled="busy==='artifact' || !artifactDir.trim()" @click="runArtifact">{{ busy==='artifact' ? tr('运行中…','Running…') : tr('分析','Analyze') }}</button>
      </div>
      <small v-if="result.artifact" class="mc-ok">{{ result.artifact }}</small>
      <small v-if="errorText.artifact" class="mc-err">{{ errorText.artifact }}</small>
    </div>

    <div class="mc-tool">
      <label>{{ tr("② 暴露端点探测（目标 URL）", "② Exposed-endpoint probe (target URL)") }}</label>
      <div class="mc-row">
        <input v-model="exposureUrl" placeholder="https://target.example.com" />
        <button class="button primary" :disabled="busy==='exposure' || !exposureUrl.trim()" @click="runExposure">{{ busy==='exposure' ? tr('运行中…','Running…') : tr('探测','Probe') }}</button>
      </div>
      <small v-if="result.exposure" class="mc-ok">{{ result.exposure }}</small>
      <small v-if="errorText.exposure" class="mc-err">{{ errorText.exposure }}</small>
    </div>

    <div class="mc-tool">
      <label>{{ tr("③ 对象存储检测（桶 URL）", "③ Cloud-bucket exposure (bucket URL)") }}</label>
      <div class="mc-row">
        <input v-model="bucketUrl" placeholder="https://bucket.oss-cn-….aliyuncs.com" />
        <button class="button primary" :disabled="busy==='bucket' || !bucketUrl.trim()" @click="runBucket">{{ busy==='bucket' ? tr('运行中…','Running…') : tr('检测','Check') }}</button>
      </div>
      <label class="mc-check"><input type="checkbox" v-model="bucketCanary" />{{ tr("写入可逆 canary 验证可写（需 target_write 授权）", "Write a reversible canary to prove write (needs target_write)") }}</label>
      <small v-if="result.bucket" class="mc-ok">{{ result.bucket }}</small>
      <small v-if="errorText.bucket" class="mc-err">{{ errorText.bucket }}</small>
    </div>

    <div class="mc-tool">
      <label>{{ tr("④ JWT 弱点分析（token + 候选密钥，每行一个）", "④ JWT weakness analysis (token + candidate secrets, one per line)") }}</label>
      <textarea v-model="jwtToken" rows="2" placeholder="eyJhbGciOi…"></textarea>
      <textarea v-model="jwtSecrets" rows="2" :placeholder="tr('候选密钥/公钥 PEM，每行一个（可留空）', 'candidate secrets / public-key PEM, one per line (optional)')"></textarea>
      <div class="mc-row">
        <button class="button primary" :disabled="busy==='jwt' || !jwtToken.trim()" @click="runJwt">{{ busy==='jwt' ? tr('运行中…','Running…') : tr('分析','Analyze') }}</button>
      </div>
      <small v-if="result.jwt" class="mc-ok">{{ result.jwt }}</small>
      <small v-if="errorText.jwt" class="mc-err">{{ errorText.jwt }}</small>
    </div>

    <div class="mc-tool">
      <label>{{ tr("⑤ 攻击扫荡（对已发现接口代码驱动跑 SQLi，不靠模型）", "⑤ Attack sweep (code-driven SQLi over discovered endpoints, no model)") }}</label>
      <div class="mc-row">
        <input v-model="sweepUrl" placeholder="http://127.0.0.1:3000" />
        <button class="button primary" :disabled="busy==='sweep' || !sweepUrl.trim()" @click="runSweep">{{ busy==='sweep' ? tr('扫荡中…','Sweeping…') : tr('扫荡','Sweep') }}</button>
      </div>
      <small class="mc-note">{{ tr("读取本任务 recon 已发现的接口，对每个查询参数/JSON 体字段直接跑 SQL 注入差异探测。需 target_write 授权。先跑过一次扫描让 recon 收集接口。", "Reads this task's recon-discovered endpoints and runs the SQL-injection differential on each query param / JSON body field directly. Needs target_write. Run a scan first so recon has collected endpoints.") }}</small>
      <small v-if="result.sweep" class="mc-ok">{{ result.sweep }}</small>
      <small v-if="errorText.sweep" class="mc-err">{{ errorText.sweep }}</small>
    </div>
  </details>
</template>

<style scoped>
.manual-caps {
  margin: 12px 0;
  padding: 10px 14px;
  border: 1px solid var(--border, #d0d0d0);
  border-radius: 10px;
  background: var(--surface, #fff);
}
.manual-caps > summary { cursor: pointer; font-weight: 600; color: var(--ink, #1a1a1a); font-size: 0.92rem; }
.mc-note { display: block; margin: 8px 0 4px; color: var(--muted, #6a6a6a); font-size: 0.78rem; line-height: 1.5; }
.mc-tool { padding: 10px 0; border-top: 1px solid var(--border, #ececec); display: flex; flex-direction: column; gap: 6px; }
.mc-tool > label { font-size: 0.84rem; font-weight: 600; color: var(--ink, #1a1a1a); }
.mc-row { display: flex; gap: 8px; }
.mc-row input { flex: 1; min-width: 0; }
.manual-caps input[type="text"], .manual-caps input:not([type]), .manual-caps textarea {
  padding: 6px 8px; border: 1px solid var(--border, #d0d0d0); border-radius: 6px; font: inherit; width: 100%; box-sizing: border-box;
}
.mc-check { display: flex; align-items: center; gap: 6px; font-size: 0.8rem; color: var(--muted, #555); }
.mc-ok { color: #2e7d32; font-size: 0.82rem; word-break: break-word; }
.mc-err { color: #b4332f; font-size: 0.82rem; word-break: break-word; }
</style>
