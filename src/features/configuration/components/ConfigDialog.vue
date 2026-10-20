<script setup lang="ts">
import ProfileSectionEditor from "./ProfileSectionEditor.vue";
import type { ConfigProfile } from "../types";
import type { ConfigProfileSection } from "../configProfileSections";
defineProps<{profile?:ConfigProfile;initialSection?:ConfigProfileSection}>();
defineEmits<{saved:[id:number];close:[]}>();
</script>
<template><ProfileSectionEditor :profile="profile" :section="initialSection ?? 'assetApis'" @saved="id => $emit('saved', id)" /></template>
