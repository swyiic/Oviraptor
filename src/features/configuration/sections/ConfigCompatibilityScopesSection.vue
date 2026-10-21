<script setup lang="ts">
import { computed, ref } from "vue";
import ConfigProfileMetadataEditor from "../components/ConfigProfileMetadataEditor.vue";
import { configurationApi } from "../api";
import type { ConfigProfile } from "../types";
import { useI18n } from "../../../i18n";

const props = defineProps<{ profiles: ConfigProfile[] }>();
const emit = defineEmits<{ changed: [id: number] }>();
const { tr } = useI18n();
const selectedId = ref<number>();
const selected = computed(() => props.profiles.find(profile => profile.id === selectedId.value) ?? props.profiles[0]);
const busy = ref(false), error = ref("");
const deleteConfirmation = ref<number>();
async function remove() {
  const profile = selected.value;
  if (!profile || profile.isDefault || busy.value) return;
  if (deleteConfirmation.value !== profile.id) { deleteConfirmation.value = profile.id; return; }
  busy.value = true; error.value = "";
  try { await configurationApi.deleteProfile(profile.id); emit("changed", profile.id); deleteConfirmation.value = undefined; }
  catch (reason) { error.value = String(reason); }
  finally { busy.value = false; }
}
</script>

<template>
  <section class="config-compatibility panel" data-testid="config-compatibility-scopes">
    <h3>{{ tr('历史兼容来源', 'Historical compatibility sources') }}</h3>
    <p class="helper">{{ tr('此处只管理需要旧方案 ID 的历史来源。普通设置在各自分类页直接编辑；不会把全部配置重新合成一个设置方案。', 'This page manages historical sources that still require a profile id. Edit normal settings on their own category pages; they are not recombined into a settings profile.') }}</p>
    <label v-if="profiles.length" class="field"><span>{{ tr('历史来源', 'Historical source') }}</span><select :value="selected?.id" :disabled="busy" @change="selectedId=Number(($event.target as HTMLSelectElement).value);deleteConfirmation=undefined"><option v-for="profile in profiles" :key="profile.id" :value="profile.id">{{profile.name}}{{profile.isDefault?tr('（兼容默认）',' (compatibility default)'):''}}</option></select></label>
    <ConfigProfileMetadataEditor v-if="selected" :key="selected.id" :profile="selected" @saved="id => $emit('changed', id)" @close="deleteConfirmation=undefined" />
    <button v-if="selected && !selected.isDefault" class="button danger compact" :disabled="busy" @click="remove">{{deleteConfirmation===selected.id?tr('确认删除历史来源','Confirm deletion'):tr('删除历史来源','Delete historical source')}}</button>
    <p v-if="!profiles.length" class="empty-state small">{{tr('未找到初始化兼容来源，请重新加载配置中心。','The initial compatibility source is missing. Reload Configuration.')}}</p>
    <p v-if="error" class="form-error" role="alert">{{error}}</p>
  </section>
</template>

<style scoped>
.config-compatibility{padding:20px}.config-compatibility h3{margin:0 0 12px}.config-compatibility .helper{font-size:12px;line-height:1.7;margin:10px 0 16px}.compatibility-metadata{margin:16px 0}
</style>
