import { onBeforeUnmount, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { AgentHostChatClient, AgentConversationFile, AgentConversationFilePreview } from './chatClient';

type Options = { client: () => AgentHostChatClient; conversationId: () => string | undefined; projectId: () => number | undefined; refreshKey: () => unknown };
export function useConversationFiles(options: Options) {
  const items = ref<AgentConversationFile[]>([]), workspacePath = ref(''), loading = ref(false), error = ref('');
  const unavailable = ref(0), truncated = ref(false), selectedFile = ref<AgentConversationFile>();
  const preview = ref<AgentConversationFilePreview>(), previewLoading = ref(false), actionBusy = ref(false), notice = ref('');
  let generation = 0, reads = 0, previews = 0, disposed = false;
  const current = (g: number, id: string, pid: number) => !disposed && g === generation && options.conversationId() === id && options.projectId() === pid;
  const failure = (e: unknown) => {
    const code = e instanceof Error ? e.message : String(e);
    if (/unavailable|not_found/.test(code)) return '文件已不存在或不属于当前会话，请刷新文件列表。';
    if (/preview_unsupported/.test(code)) return '此文件不支持文本预览，请在 Finder 中查看。';
    if (/path_denied|permission|scope/.test(code)) return '无法访问此文件，请检查工作目录及文件权限。';
    return `文件操作失败，可重试。${/^[a-z_\d]+$/.test(code) ? `（${code}）` : ''}`;
  };
  async function refresh() {
    const id = options.conversationId(), pid = options.projectId(), g = generation, sequence = ++reads;
    if (!id || pid === undefined || !options.client().conversationFiles) return;
    loading.value = true;
    try {
      const result = await options.client().conversationFiles(id, pid);
      if (!current(g, id, pid) || sequence !== reads) return;
      if (result.projectId !== pid || result.conversationId !== id) throw Error('host_chat_file_scope_mismatch');
      items.value = result.items; workspacePath.value = result.workspacePath;
      unavailable.value = result.unavailable; truncated.value = result.truncated; error.value = '';
      if (selectedFile.value && !result.items.some(item => item.path === selectedFile.value?.path)) {
        closePreview(); notice.value = '该文件已不存在或无法访问，预览已关闭；历史写入记录保留。';
      }
    } catch (e) { if (current(g, id, pid) && sequence === reads) error.value = failure(e); }
    finally { if (current(g, id, pid) && sequence === reads) loading.value = false; }
  }
  function closePreview() { ++previews; selectedFile.value = undefined; preview.value = undefined; previewLoading.value = false; }
  async function fileAction(path: string, action: 'open' | 'reveal') {
    const id = options.conversationId(), pid = options.projectId(), g = generation;
    const item = items.value.find(item => item.path === path);
    if (!id || pid === undefined || actionBusy.value) return;
    if (!item) { error.value = '文件已不存在或不属于当前会话，请刷新文件列表。'; return; }
    if (action === 'open' && !item.openable) { error.value = '此文件请使用文本预览或在 Finder 中查看。'; return; }
    actionBusy.value = true; notice.value = ''; error.value = '';
    try {
      await invoke('agent_host_chat_file_action', { projectId: pid, conversationId: id, relativePath: path, action });
      if (current(g, id, pid)) notice.value = action === 'open' ? '已交给默认应用打开。' : '已在 Finder 中显示文件。';
    } catch (e) { if (current(g, id, pid)) error.value = failure(e); }
    finally { if (current(g, id, pid)) actionBusy.value = false; }
  }
  async function showPreview(path: string) {
    const id = options.conversationId(), pid = options.projectId(), g = generation;
    const item = items.value.find(item => item.path === path);
    if (!id || pid === undefined || !item) return;
    if (!item.previewable) { await fileAction(path, item.openable ? 'open' : 'reveal'); return; }
    const sequence = ++previews; selectedFile.value = item; preview.value = undefined; previewLoading.value = true; error.value = '';
    try {
      const result = await options.client().previewConversationFile(id, pid, path);
      if (!current(g, id, pid) || sequence !== previews) return;
      if (result.path !== path) throw Error('host_chat_file_scope_mismatch');
      preview.value = result;
    } catch (e) { if (current(g, id, pid) && sequence === previews) error.value = failure(e); }
    finally { if (current(g, id, pid) && sequence === previews) previewLoading.value = false; }
  }
  watch([options.conversationId, options.projectId, options.client], () => {
    ++generation; ++reads; closePreview(); items.value = []; workspacePath.value = ''; error.value = ''; notice.value = '';
    unavailable.value = 0; truncated.value = false; loading.value = false; actionBusy.value = false;
    void refresh();
  }, { immediate: true, flush: 'sync' });
  watch(options.refreshKey, () => { void refresh(); });
  onBeforeUnmount(() => { disposed = true; ++generation; ++reads; ++previews; });
  return { items, workspacePath, loading, error, unavailable, truncated, selectedFile, preview, previewLoading, actionBusy, notice, refresh, showPreview, closePreview, fileAction };
}
export type ConversationFilesState = ReturnType<typeof useConversationFiles>;
