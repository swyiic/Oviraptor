<script setup lang="ts">
import ConfigProfileMetadataEditor from "./ConfigProfileMetadataEditor.vue";
import type { ConfigProfile } from "../types";
defineProps<{profile:ConfigProfile}>();
defineEmits<{saved:[id:number];close:[]}>();
</script>
<template><ConfigProfileMetadataEditor :profile="profile" @saved="id => $emit('saved', id)" @close="$emit('close')" /></template>
