<script setup lang="ts">
import { onUnmounted, reactive, ref, watch } from "vue";
import { useI18n } from "../../../i18n";
import { configurationApi } from "../api";
import type { ConfigProfile } from "../types";

const props = defineProps<{ profile: ConfigProfile }>();
const emit = defineEmits<{ close: []; saved: [id: number] }>();
const { tr } = useI18n();
const metadata = () => ({ name: props.profile.name, description: props.profile.description, isDefault: props.profile.isDefault });
const form = reactive(metadata());
let expected = metadata(), generation = 0, disposed = false;
const busy = ref(false);
const error = ref("");

watch(() => props.profile, (profile, previous) => {
  const current = metadata();
  if (profile.id === previous?.id && JSON.stringify(current) === JSON.stringify(expected)) return;
  if (profile.id === previous?.id && JSON.stringify(form) !== JSON.stringify(expected) && JSON.stringify(form) !== JSON.stringify(current)) {
    error.value = tr("历史来源信息已更新，当前草稿保留。放弃草稿可加载最新信息。", "Historical source details changed. Your draft is retained; discard it to load the latest details.");
    return;
  }
  generation++; expected = current; Object.assign(form, expected);
  busy.value = false; error.value = "";
});
function discard() {
  if (busy.value) return;
  generation++; expected = metadata(); Object.assign(form, expected); error.value = "";
  emit("close");
}
onUnmounted(() => { disposed = true; generation++; });

async function save() {
  if (busy.value) return;
  if (!form.name.trim()) { error.value = tr("请输入历史来源名称", "Enter a profile name"); return; }
  if (expected.isDefault && !form.isDefault) { error.value = tr("请将其他方案设为默认，不能直接取消当前默认方案", "Choose a replacement default profile before switching"); return; }
  const profileId = props.profile.id, request = ++generation;
  const input = { profileId, name: form.name.trim(), description: form.description.trim(), isDefault: form.isDefault, expected: { ...expected } };
  busy.value = true; error.value = "";
  try {
    const id = await configurationApi.saveProfileMetadata(input);
    if (!disposed && request === generation && props.profile.id === profileId) emit("saved", id);
  } catch (reason) {
    if (!disposed && request === generation && props.profile.id === profileId) error.value = String((reason as Error)?.message || reason);
  } finally { if (!disposed && request === generation) busy.value = false; }
}
</script>

<template>
  <section class="compatibility-metadata" data-testid="compatibility-metadata">
    <form class="form-stack" @submit.prevent="save">
      <p class="helper">{{ tr('在这里保存历史来源名称、说明与默认选择。模型、账号、策略和规则分别在对应分类保存。', 'Change the historical source name and description. The compatibility default affects legacy entries using that source; each category still saves independently.') }}</p>
      <label class="field"><span>{{ tr('历史来源名称', 'Historical source name') }}</span><input v-model="form.name" :disabled="busy" autofocus /></label>
      <label class="field"><span>{{ tr('说明', 'Description') }}</span><textarea v-model="form.description" :disabled="busy" rows="3"></textarea></label>
      <label class="check-field"><input v-model="form.isDefault" type="checkbox" :disabled="busy || props.profile.isDefault" />{{ tr('设为兼容默认来源', 'Use as the compatibility default') }}</label>
      <p v-if="props.profile.isDefault" class="helper">{{ tr('切换默认方案时，请在另一份方案中选择“设为兼容默认来源”。', 'To switch defaults, select “Use as the compatibility default” on the replacement profile.') }}</p>
      <p class="helper">{{ tr('兼容默认只影响仍显式使用历史来源的旧入口，不切换任何独立分类默认。新的模型、资产与扫描设置从各分类读取，已有任务保持其快照。', 'The compatibility default affects only legacy entries explicitly using historical sources. It does not switch independent category defaults. New model, asset and scan settings come from their category; existing tasks retain their snapshots.') }}</p>
      <p v-if="error" class="form-error" role="alert">{{ error }}</p>
    </form>
    <footer class="hero-actions">
      <button class="button ghost" type="button" :disabled="busy" @click="discard">{{ tr('放弃未保存信息', 'Discard unsaved details') }}</button>
      <button class="button primary" type="button" :disabled="busy || !form.name.trim()" @click="save">{{ busy ? tr('保存中…', 'Saving…') : tr('保存方案信息', 'Save profile details') }}</button>
    </footer>
  </section>
</template>
