<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";

const props = defineProps<{ scanId: string; targets: { url: string }[] }>();
const { tr } = useI18n();
type Setup = Awaited<ReturnType<typeof sentinelApi.getAuthorizationControlSetup>>;
const targetUrl = ref("");
const setup = ref<Setup>();
const loading = ref(false);
const saving = ref(false);
const error = ref("");
const notice = ref("");
const contractKey = ref("");
const ownerObjectUrl = ref("");
const testerControlUrl = ref("");
const objectQueryKey = ref("");
const ownerObjectValue = ref("");
const testerObjectValue = ref("");
const responseObjectPointer = ref("");
const ownerIdentity = ref("");
const testerIdentity = ref("");
let requestVersion = 0;

watch(() => [props.scanId, props.targets.map(row => row.url).join("\n")], () => {
  targetUrl.value = props.targets[0]?.url || "";
}, { immediate: true });

watch(() => [props.scanId, targetUrl.value], async () => {
  const version = ++requestVersion;
  setup.value = undefined;
  error.value = "";
  notice.value = "";
  if (!targetUrl.value) return;
  loading.value = true;
  try {
    const result = await sentinelApi.getAuthorizationControlSetup(props.scanId, targetUrl.value);
    if (version !== requestVersion) return;
    setup.value = result;
    ownerIdentity.value = result.identityIds[0] || "";
    testerIdentity.value = result.identityIds[1] || "";
  } catch (reason) {
    if (version === requestVersion) error.value = String(reason);
  } finally {
    if (version === requestVersion) loading.value = false;
  }
}, { immediate: true });

const canSave = computed(() => !!setup.value && !saving.value &&
  !!contractKey.value.trim() && !!ownerObjectUrl.value.trim() && !!testerControlUrl.value.trim() &&
  !!objectQueryKey.value.trim() && !!ownerObjectValue.value.trim() && !!testerObjectValue.value.trim() &&
  !!responseObjectPointer.value.trim() && !!ownerIdentity.value && !!testerIdentity.value &&
  ownerIdentity.value !== testerIdentity.value);

async function save() {
  if (!canSave.value || !setup.value) return;
  const savedScan = props.scanId;
  const savedTarget = targetUrl.value;
  const attempt = setup.value.attemptNumber;
  saving.value = true;
  error.value = "";
  notice.value = "";
  try {
    await sentinelApi.saveAuthorizationControl({
      scanId: savedScan, attemptNumber: attempt, targetUrl: savedTarget,
      contractKey: contractKey.value.trim(), ownerObjectUrl: ownerObjectUrl.value.trim(),
      testerControlUrl: testerControlUrl.value.trim(), objectQueryKey: objectQueryKey.value.trim(),
      ownerObjectValue: ownerObjectValue.value.trim(), testerObjectValue: testerObjectValue.value.trim(),
      responseObjectPointer: responseObjectPointer.value.trim(), ownerIdentity: ownerIdentity.value,
      testerIdentity: testerIdentity.value,
    });
    if (savedScan !== props.scanId || savedTarget !== targetUrl.value) return;
    const refreshed = await sentinelApi.getAuthorizationControlSetup(savedScan, savedTarget);
    if (savedScan !== props.scanId || savedTarget !== targetUrl.value) return;
    setup.value = refreshed;
    notice.value = tr("控制组已登记且不可修改；任务开始后将由独立授权 Agent 执行三侧 GET，仍须经 Reviewer 审核。", "Control group registered and locked. Once the task starts, an independent authorization Agent runs the three-sided GETs; Reviewer review is still required.");
  } catch (reason) {
    if (savedScan === props.scanId && savedTarget === targetUrl.value) error.value = String(reason);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <details class="task-technical authorization-control-form">
    <summary>{{ tr("越权对象控制组（启动前登记）", "Access-control object controls (register before start)") }}</summary>
    <p>{{ tr("仅限已授权的只读 GET 对象。登记的是操作员声明，不是验证结果；任务开始后，独立授权 Agent 才能按三侧证据执行，并由 Reviewer 审核。", "Authorized read-only GET objects only. A registration is an operator claim, not a verification result. Once the task starts, an independent authorization Agent runs it on three-sided evidence, and a Reviewer reviews it.") }}</p>
    <label class="field"><span>{{ tr("任务目标", "Task target") }}</span><select v-model="targetUrl"><option v-for="row in targets" :key="row.url" :value="row.url">{{ row.url }}</option></select></label>
    <p v-if="loading">{{ tr("正在核验任务身份与下一次 attempt…", "Checking task identities and the next attempt…") }}</p>
    <p v-if="error" role="alert">{{ error }}</p>
    <template v-if="setup">
      <p>{{ tr(`下一次 attempt：${setup.attemptNumber}；身份句柄仅用于选择，不展示凭证。`, `Next attempt: ${setup.attemptNumber}. Identity handles are for selection only; credentials are never shown.`) }}</p>
      <p v-if="setup.contractKeys.length">{{ tr(`已登记（不可覆盖）：${setup.contractKeys.join("、")}`, `Registered (cannot be overwritten): ${setup.contractKeys.join(", ")}`) }}</p>
      <label class="field"><span>{{ tr("验证合同 key", "Verification contract key") }}</span><input v-model="contractKey" :placeholder="tr('与实际执行合同完全一致', 'Must exactly match the executed contract')" /></label>
      <label class="field"><span>{{ tr("对象所有者 A", "Object owner A") }}</span><select v-model="ownerIdentity"><option v-for="id in setup.identityIds" :key="id" :value="id">{{ id }}</option></select></label>
      <label class="field"><span>{{ tr("测试身份 B", "Test identity B") }}</span><select v-model="testerIdentity"><option v-for="id in setup.identityIds" :key="id" :value="id">{{ id }}</option></select></label>
      <label class="field"><span>{{ tr("A 的对象 X URL", "Object X URL (owned by A)") }}</span><input v-model="ownerObjectUrl" type="url" placeholder="https://target/api/orders?id=X" /></label>
      <label class="field"><span>{{ tr("B 的合法对象 Y URL", "Legitimate object Y URL (owned by B)") }}</span><input v-model="testerControlUrl" type="url" placeholder="https://target/api/orders?id=Y" /></label>
      <label class="field"><span>{{ tr("唯一对象 query 键", "Unique object query key") }}</span><input v-model="objectQueryKey" placeholder="id" /></label>
      <label class="field"><span>{{ tr("对象 X 的 query 值", "Query value for object X") }}</span><input v-model="ownerObjectValue" /></label>
      <label class="field"><span>{{ tr("对象 Y 的 query 值", "Query value for object Y") }}</span><input v-model="testerObjectValue" /></label>
      <label class="field"><span>{{ tr("响应对象标识 JSON Pointer", "Response object ID JSON Pointer") }}</span><input v-model="responseObjectPointer" placeholder="/order/id" /></label>
      <button class="button secondary" type="button" :disabled="!canSave" @click="save">{{ saving ? tr("正在登记…", "Registering…") : tr("登记控制组", "Register control group") }}</button>
      <p v-if="notice" role="status">{{ notice }}</p>
    </template>
  </details>
</template>
