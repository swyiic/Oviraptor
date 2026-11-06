<script setup lang="ts">
import { AlertTriangle } from "@lucide/vue";
import { useI18n } from "../../i18n";

const { t } = useI18n();
withDefaults(defineProps<{
  title: string;
  detail?: string;
  confirmText?: string;
  busyText?: string;
  cancelText?: string;
  tone?: "danger" | "warning" | "secondary";
  busy?: boolean;
}>(), { detail: "", tone: "danger", busy: false });

defineEmits<{ confirm: []; cancel: [] }>();
</script>

<template>
  <div class="inline-confirm" :class="`tone-${tone}`" role="alert">
    <AlertTriangle :size="16" />
    <div><strong>{{title}}</strong><p v-if="detail">{{detail}}</p></div>
    <button type="button" class="button ghost compact" :disabled="busy" @click="$emit('cancel')">{{cancelText || t.common.cancel}}</button>
    <button type="button" :class="`button ${tone} compact`" :disabled="busy" @click="$emit('confirm')">{{busy ? (busyText || t.common.deleting) : (confirmText || t.common.confirmDelete)}}</button>
  </div>
</template>
