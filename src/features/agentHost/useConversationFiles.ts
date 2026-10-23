import { onBeforeUnmount, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { AgentHostChatClient, AgentConversationFile, AgentConversationFilePreview } from './chatClient';

type Options = { client: () => AgentHostChatClient; conversationId: () => string | undefined; projectId: () => number | undefined; refreshKey: () => unknown };
export function useConversationFiles(options: Options) {
  const items = ref<AgentConversationFile[]>([]), workspacePath = ref(''), loading = ref(false), error = ref('');
  const unavailable = ref(0), truncated = ref(false), selectedFile = ref<AgentConversationFile>();
  const preview = ref<AgentConversationFilePreview>(), previewLoading = ref(false), actionBusy = ref(false), notice = ref('');
  const references=ref<Array<{path:string;absolutePath:string}>>([]),hasMore=ref(false),versionOptions=ref<Array<{versionId:string;createdAt:string;size:number;kind:string}>>([]),selectedVersion=ref(''),versionsMore=ref(false);
  let nextFile:string|undefined,nextVersion:string|undefined;
  let generation = 0, reads = 0, previews = 0, disposed = false;
  const current = (g: number, id: string, pid: number) => !disposed && g === generation && options.conversationId() === id && options.projectId() === pid;
  const failure = (e: unknown) => {
    const code = e instanceof Error ? e.message : String(e);
    if (/unavailable|not_found/.test(code)) return '文件已不存在或不属于当前会话，请刷新文件列表。';
    if (/preview_unsupported/.test(code)) return '此文件不支持文本预览，请在 Finder 中查看。';
    if (/path_denied|permission|scope/.test(code)) return '无法访问此文件，请检查工作目录及文件权限。';
    return `文件操作失败，可重试。${/^[a-z_\d]+$/.test(code) ? `（${code}）` : ''}`;
  };
  async function refresh(more=false) {
    const id = options.conversationId(), pid = options.projectId(), g = generation, sequence = ++reads;
    if (!id || pid === undefined || !options.client().conversationFiles) return;
    loading.value = true;
    try {
      const result = await options.client().conversationFiles(id, pid,more?nextFile:undefined);
      if (!current(g, id, pid) || sequence !== reads) return;
      if (result.projectId !== pid || result.conversationId !== id) throw Error('host_chat_file_scope_mismatch');
      const previous=selectedFile.value;
      if(more){const known=new Set(items.value.map(f=>f.path));items.value=[...items.value,...result.items.filter(f=>!known.has(f.path))];}else items.value=result.items;
      const refs=result.references??items.value;if(JSON.stringify(references.value)!==JSON.stringify(refs.map(f=>({path:f.path,absolutePath:f.absolutePath}))))references.value=refs;
      hasMore.value=result.hasMore??false;nextFile=result.nextCursor??undefined;workspacePath.value = result.workspacePath;
      let updated=previous&&items.value.find(f=>f.path===previous.path);
      if(previous&&!updated&&!more&&references.value.some(f=>f.path===previous.path)&&options.client().conversationFile){try{updated=await options.client().conversationFile(id,pid,previous.path);if(!current(g,id,pid)||sequence!==reads)return;if(updated.path!==previous.path)throw Error('host_chat_file_scope_mismatch');items.value=[...items.value,updated];}catch(e){if(!current(g,id,pid)||sequence!==reads)return;closePreview();notice.value=failure(e);}}
      if(updated&&previous&&JSON.stringify(updated)!==JSON.stringify(previous)&&!selectedVersion.value)void showPreview(updated.path);
      unavailable.value = result.unavailable; truncated.value = result.truncated; error.value = '';
      if (selectedFile.value && !more && !items.value.some(item => item.path === selectedFile.value?.path) && !references.value.some(item=>item.path===selectedFile.value?.path)) {
        closePreview(); notice.value = '该文件已不存在或无法访问，预览已关闭；历史写入记录保留。';
      }
    } catch (e) { if (current(g, id, pid) && sequence === reads) error.value = failure(e); }
    finally { if (current(g, id, pid) && sequence === reads) loading.value = false; }
  }
  function closePreview() { ++previews;selectedVersion.value='';versionOptions.value=[];versionsMore.value=false; selectedFile.value = undefined; preview.value = undefined; previewLoading.value = false; }
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
  async function showPreview(path: string, versionId = '') {
    const id = options.conversationId(), pid = options.projectId(), g = generation;
    if(!id||pid===undefined)return;
    const sequence=++previews;
    let item=items.value.find(item=>item.path===path);
    if(!item&&references.value.some(f=>f.path===path)&&options.client().conversationFile){try{const value=await options.client().conversationFile(id,pid,path);if(!current(g,id,pid)||sequence!==previews||value.path!==path)return;item=value;items.value=[...items.value,value];}catch(e){if(current(g,id,pid))error.value=failure(e);return;}}
    if(!item)return;
    if (!item.previewable) { await fileAction(path, item.openable ? 'open' : 'reveal'); return; }
    selectedVersion.value=versionId; selectedFile.value = item; preview.value = undefined; previewLoading.value = true; error.value = '';
    try {
      const result = await options.client().previewConversationFile(id, pid, path,versionId||undefined);
      if (!current(g, id, pid) || sequence !== previews) return;
      if (result.path !== path) throw Error('host_chat_file_scope_mismatch');
      preview.value = result;
      if(options.client().fileVersions){const versions=await options.client().fileVersions(id,pid,path);if(current(g,id,pid)&&sequence===previews){versionOptions.value=versions.items;versionsMore.value=versions.hasMore;nextVersion=versions.nextCursor??undefined;}}
    } catch (e) { if (current(g, id, pid) && sequence === previews) error.value = failure(e); }
    finally { if (current(g, id, pid) && sequence === previews) previewLoading.value = false; }
  }
  async function readMore(){const id=options.conversationId(),pid=options.projectId(),g=generation,path=selectedFile.value?.path,previous=preview.value,version=selectedVersion.value,seq=previews;if(!id||pid===undefined||!path||!previous?.truncated||previous.nextOffset===undefined||previewLoading.value)return;previewLoading.value=true;try{const result=await options.client().previewConversationFile(id,pid,path,version||undefined,previous.nextOffset);if(!current(g,id,pid)||seq!==previews)return;if(result.path!==path||result.modifiedAt!==previous.modifiedAt||result.size!==previous.size){error.value='文件已改变，请重新打开预览，避免混合不同版本。';return;}preview.value={...result,content:previous.content+result.content};}catch(e){if(current(g,id,pid)&&seq===previews)error.value=failure(e);}finally{if(current(g,id,pid)&&seq===previews)previewLoading.value=false;}}
  async function loadMoreVersions(){const id=options.conversationId(),pid=options.projectId(),g=generation,path=selectedFile.value?.path;if(!id||pid===undefined||!path||!nextVersion)return;try{const v=await options.client().fileVersions(id,pid,path,nextVersion);if(current(g,id,pid)&&selectedFile.value?.path===path){versionOptions.value=[...versionOptions.value,...v.items];versionsMore.value=v.hasMore;nextVersion=v.nextCursor??undefined;}}catch(e){if(current(g,id,pid))error.value=failure(e);}}
  watch([options.conversationId, options.projectId, options.client], () => {
    ++generation; ++reads; closePreview(); items.value = [];references.value=[];hasMore.value=false;nextFile=undefined; workspacePath.value = ''; error.value = ''; notice.value = '';
    unavailable.value = 0; truncated.value = false; loading.value = false; actionBusy.value = false;
    void refresh();
  }, { immediate: true, flush: 'sync' });
  watch(options.refreshKey, () => { void refresh(); });
  onBeforeUnmount(() => { disposed = true; ++generation; ++reads; ++previews; });
  return { readMore,references,hasMore,versionOptions,selectedVersion,versionsMore,loadMoreVersions,items, workspacePath, loading, error, unavailable, truncated, selectedFile, preview, previewLoading, actionBusy, notice, refresh, showPreview, closePreview, fileAction };
}
export type ConversationFilesState = ReturnType<typeof useConversationFiles>;
