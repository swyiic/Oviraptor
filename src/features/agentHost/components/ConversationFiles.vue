<script setup lang="ts">
import { FileText, FolderOpen, RefreshCw, X, ExternalLink } from '@lucide/vue';
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { useI18n } from '../../../i18n';
import type { ConversationFilesState } from '../useConversationFiles';
import { messageTimestamp } from '../../conversations/messageTime';
import MessageCopyButton from '../../conversations/MessageCopyButton.vue';
import { nextTick, onBeforeUnmount, ref, watch } from 'vue';
const props = defineProps<{ state: ConversationFilesState }>();
const { tr } = useI18n();
const previewElement = ref<HTMLElement>(), closeButton = ref<HTMLButtonElement>();
let previousFocus: HTMLElement | undefined;
watch(() => props.state.selectedFile.value, async (file, previous) => {
  if (!file) { previousFocus?.focus?.(); previousFocus = undefined; return; }
  if (!previous && typeof document !== 'undefined') previousFocus = document.activeElement as HTMLElement;
  await nextTick(); if (props.state.selectedFile.value) closeButton.value?.focus?.();
});
function previewKey(event: KeyboardEvent) {
  if (event.key === 'Escape') { event.stopPropagation(); props.state.closePreview(); return; }
  if (event.key !== 'Tab') return;
  const focusable = [...(previewElement.value?.querySelectorAll<HTMLElement>('button:not(:disabled),pre[tabindex]') || [])];
  const first = focusable[0], last = focusable[focusable.length - 1];
  if (!first || typeof document === 'undefined') return;
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
}
onBeforeUnmount(() => { previousFocus?.focus?.(); });
function size(value: number) { return value < 1024 ? `${value} B` : value < 1048576 ? `${(value / 1024).toFixed(1)} KB` : `${(value / 1048576).toFixed(1)} MB`; }
function modified(value: string) { return messageTimestamp(value, tr('zh-CN', 'en-GB'))?.label || tr('时间未知', 'Time unknown'); }
</script>
<template>
  <section v-if="state.items.value.length || state.error.value || state.unavailable.value" class="chat-files" aria-label="会话文件" data-testid="conversation-files">
    <header><strong>{{ tr('本会话文件', 'Conversation files') }} <span>{{state.items.value.length}}</span></strong><button v-refresh-feedback class="files-icon" type="button" :disabled="state.loading.value" :title="tr('刷新文件', 'Refresh files')" :aria-label="tr('刷新文件', 'Refresh files')" @click="state.refresh"><RefreshCw :size="14" :class="{ spinning: state.loading.value }" /></button></header>
    <p class="files-caption">{{tr('点击文件名预览当前版本；文件保存在项目共享目录，其他会话可能修改同一路径，此处不是历史快照。', 'Preview the current file. The project directory is shared; other chats may modify the same path. This is not a historical snapshot.')}}</p>
    <div class="files-grid"><article v-for="file in state.items.value" :key="file.path" class="file-card">
      <FileText :size="19" class="file-mark" /><div class="file-info"><button type="button" class="file-name" :title="file.absolutePath" @click="state.showPreview(file.path)">{{file.name}}</button><small>{{size(file.size)}} · {{modified(file.modifiedAt)}}</small><code :title="file.absolutePath">{{file.path}}</code></div>
      <button class="files-icon" type="button" :disabled="state.actionBusy.value" :title="tr('在 Finder 中显示', 'Reveal in Finder')" :aria-label="tr(`在 Finder 中显示 ${file.name}`, `Reveal ${file.name} in Finder`)" @click="state.fileAction(file.path, 'reveal')"><FolderOpen :size="16" /></button>
    </article></div>
    <p v-if="state.truncated.value || state.unavailable.value" class="files-caption" role="status"><span v-if="state.truncated.value">{{tr('仅展示最近的文件写入记录。', 'Showing recent file writes.')}}</span><span v-if="state.unavailable.value"> {{state.unavailable.value}} {{tr('条历史文件已不存在或无法安全访问，原记录保留。', 'historical files are missing or inaccessible; records are retained.')}}</span></p>
    <p v-if="state.error.value" class="files-error" role="alert">{{state.error.value}} <button type="button" @click="state.refresh">{{tr('重试', 'Retry')}}</button></p>
    <p v-if="state.notice.value" class="files-caption" role="status">{{state.notice.value}}</p>
    <div v-if="state.selectedFile.value" class="file-preview-backdrop" @click.self="state.closePreview" @keydown="previewKey">
      <section ref="previewElement" class="file-preview" role="dialog" aria-modal="true" :aria-label="state.selectedFile.value.name" tabindex="-1">
        <header><div><strong>{{state.selectedFile.value.name}}</strong><p class="file-location">{{state.selectedFile.value.absolutePath}}</p></div><button ref="closeButton" type="button" class="files-icon" :aria-label="tr('关闭预览', 'Close preview')" @click="state.closePreview"><X :size="18" /></button></header>
        <div class="preview-actions"><MessageCopyButton :source="state.selectedFile.value.absolutePath" :label="tr('复制路径', 'Copy path')" /><button type="button" :disabled="state.actionBusy.value" @click="state.fileAction(state.selectedFile.value.path, 'reveal')"><FolderOpen :size="14" />{{tr('在 Finder 中显示', 'Reveal in Finder')}}</button><button v-if="state.selectedFile.value.openable" type="button" :disabled="state.actionBusy.value" @click="state.fileAction(state.selectedFile.value.path, 'open')"><ExternalLink :size="14" />{{tr('默认应用打开', 'Open in default app')}}</button></div>
        <p class="files-caption">{{tr('当前文件内容；同一项目的其他会话可能已修改，不是生成时的历史快照。', 'Current file contents; another project chat may have modified this file. This is not its original snapshot.')}}</p>
        <p v-if="state.previewLoading.value" class="files-caption" role="status">{{tr('正在读取文件…', 'Reading file…')}}</p>
        <p v-if="state.error.value" class="files-error" role="alert">{{state.error.value}}</p>
        <template v-if="state.preview.value"><p class="files-caption">{{size(state.preview.value.size)}} · {{tr('修改于', 'Modified')}} {{modified(state.preview.value.modifiedAt)}}<span v-if="state.preview.value.truncated"> · {{tr('仅预览前 64 KB，完整文件保留在本地', 'Preview limited to 64 KB; full file is saved locally')}}</span></p><pre tabindex="0">{{state.preview.value.content}}</pre></template>
      </section>
    </div>
  </section>
</template>
<style scoped>
.chat-files{border:1px solid var(--line,#e1e3e7);border-radius:12px;padding:14px 16px;margin:18px 0 22px;min-width:0}.chat-files header{display:flex;justify-content:space-between;align-items:center;gap:12px}.chat-files header strong{font-size:12px}.chat-files header strong span{font-weight:400;color:var(--app-muted,#687078);margin-left:5px}.files-caption{font-size:11px;color:var(--app-muted,#687078);margin:7px 0;line-height:1.6}.files-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,265px),1fr));gap:9px}.file-card{display:flex;align-items:center;gap:10px;border:1px solid var(--line,#e1e3e7);border-radius:9px;padding:11px;min-width:0}.file-mark{color:var(--blue,#416b9c);flex-shrink:0}.file-info{flex:1;min-width:0;display:flex;flex-direction:column;gap:3px}.file-name{display:block;border:0;background:none;padding:0;color:var(--blue,#416b9c);font-size:12px;font-weight:600;text-align:left;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;cursor:pointer}.file-info small{font-size:10px;color:var(--app-muted,#687078)}.file-info code{font-size:10px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--app-muted,#687078)}.files-icon{display:inline-flex;align-items:center;justify-content:center;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:transparent;padding:6px;color:inherit;flex-shrink:0;cursor:pointer}.files-error{color:var(--red,#b63f3f);font-size:12px;overflow-wrap:anywhere}.file-preview-backdrop{position:fixed;inset:0;z-index:1100;background:#0005;display:flex;align-items:center;justify-content:center;padding:24px}.file-preview{width:min(920px,100%);max-height:88vh;display:flex;flex-direction:column;min-width:0;padding:20px;background:var(--app-surface,#fff);border:1px solid var(--line,#e1e3e7);border-radius:14px;box-shadow:0 15px 60px #0003}.file-preview header>div{min-width:0}.file-location{font-size:11px;color:var(--app-muted,#687078);overflow-wrap:anywhere;margin:6px 0}.preview-actions{display:flex;flex-wrap:wrap;gap:7px;margin:12px 0}.preview-actions button{display:inline-flex;align-items:center;gap:5px;font-size:11px;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:transparent;padding:6px 9px;color:inherit;cursor:pointer}.file-preview pre{min-height:90px;overflow:auto;white-space:pre;margin:8px 0 0;background:var(--app-bg,#f6f7f9);border:1px solid var(--line,#e1e3e7);border-radius:8px;padding:14px;font:12px/1.7 ui-monospace,SFMono-Regular,Menlo,monospace}.spinning{animation:file-spin .8s linear infinite}@keyframes file-spin{to{transform:rotate(360deg)}}@media(max-width:600px){.file-preview-backdrop{padding:12px}.file-preview{padding:14px;max-height:92vh}.chat-files{padding:12px}}
</style>
