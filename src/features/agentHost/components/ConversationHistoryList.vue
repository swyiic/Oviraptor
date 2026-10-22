<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { Ellipsis, FolderOpen, MessageSquare, Pencil, Pin, PinOff, Trash2, X } from '@lucide/vue';
import { useI18n } from '../../../i18n';
import type { AgentHostConversation } from '../chatClient';
import { conversationActivityAt, conversationHistoryTime } from '../conversationHistory';

const props = defineProps<{
  items: AgentHostConversation[]; selectedId?: string; projectId?: number; loading?: boolean; deletingDisabled?: boolean; emptyLabel: string;
  updateConversation: (id: string, patch: { title?: string; pinned?: boolean }) => Promise<void>;
  revealConversation: (id: string) => Promise<string>;
  removeConversation: (id: string) => Promise<boolean>;
}>();
const emit = defineEmits<{ select: [id: string] }>();
const { tr, locale } = useI18n();
const menu = ref<{ id: string; x: number; y: number }>();
const dialog = ref<{ id: string; kind: 'rename' | 'delete'; title: string }>();
const titleDraft = ref(''), notice = ref(''), saving = ref(false), menuElement = ref<HTMLElement>(), dialogElement = ref<HTMLElement>(), renameInput = ref<HTMLInputElement>();
let scopeGeneration = 0, disposed = false;
let menuTrigger: HTMLElement | undefined, previousFocus: HTMLElement | undefined;
const groups = computed(() => [
  { label: tr('置顶', 'Pinned'), items: props.items.filter(item => item.pinned) },
  { label: tr('最近对话', 'Recent chats'), items: props.items.filter(item => !item.pinned) },
].filter(group => group.items.length));
const menuItem = computed(() => props.items.find(item => item.id === menu.value?.id));
function label(item?: AgentHostConversation) { return !item?.title || ['New conversation', '新对话', '新對話'].includes(item.title) ? tr('新对话', 'New conversation') : item.title; }
function time(item: AgentHostConversation) { return conversationHistoryTime(conversationActivityAt(item), locale?.value || 'zh'); }
function closeMenu(restore = false) { menu.value = undefined; if (restore && menuTrigger?.isConnected) menuTrigger.focus(); }
async function openMenu(event: MouseEvent | KeyboardEvent, item: AgentHostConversation) {
  event.preventDefault(); event.stopPropagation(); if (props.loading || saving.value) return;
  menuTrigger = typeof HTMLElement !== 'undefined' && event.currentTarget instanceof HTMLElement ? event.currentTarget : undefined;
  if (menuTrigger?.tagName !== 'BUTTON') menuTrigger = menuTrigger?.querySelector<HTMLButtonElement>('.chat-history-more') || menuTrigger;
  const bounds = menuTrigger?.getBoundingClientRect();
  const width = typeof window === 'undefined' ? 1024 : window.innerWidth, height = typeof window === 'undefined' ? 768 : window.innerHeight;
  const x = event.type === 'contextmenu' && 'clientX' in event ? event.clientX : bounds?.right || 20;
  const y = event.type === 'contextmenu' && 'clientY' in event ? event.clientY : bounds?.bottom || 20;
  menu.value = { id: item.id, x: Math.max(8, Math.min(x, width - 232)), y: Math.max(8, Math.min(y, height - 205)) };
  notice.value = ''; const generation = scopeGeneration; await nextTick();
  if (!disposed && generation === scopeGeneration && menu.value?.id === item.id) menuElement.value?.querySelector<HTMLButtonElement>('button:not([disabled])')?.focus();
}
function rowKey(event: KeyboardEvent, item: AgentHostConversation) {
  if (event.key === 'ContextMenu' || event.shiftKey && event.key === 'F10') void openMenu(event, item);
}
function menuKey(event: KeyboardEvent) {
  if (event.key === 'Escape') { event.preventDefault(); closeMenu(true); return; }
  if (event.key === 'Tab') { closeMenu(true); return; }
  if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
  event.preventDefault(); const buttons = Array.from(menuElement.value?.querySelectorAll<HTMLButtonElement>('button:not([disabled])') || []); if (!buttons.length) return;
  const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
  buttons[next]?.focus();
}
async function openDialog(kind: 'rename' | 'delete') {
  const item = menuItem.value; if (!item || saving.value || kind === 'delete' && props.deletingDisabled) return;
  previousFocus = menuTrigger; dialog.value = { id: item.id, kind, title: label(item) }; titleDraft.value = item.title || ''; notice.value = ''; closeMenu();
  const generation = scopeGeneration; await nextTick(); if (disposed || generation !== scopeGeneration || !dialog.value) return;
  if (kind === 'rename') { renameInput.value?.focus(); renameInput.value?.select(); } else dialogElement.value?.querySelector<HTMLButtonElement>('button')?.focus();
}
function closeDialog() {
  if (saving.value) return; dialog.value = undefined; notice.value = '';
  if (previousFocus?.isConnected) previousFocus.focus();
}
function dialogKey(event: KeyboardEvent) {
  if (event.isComposing) return;
  if (event.key === 'Escape') { event.preventDefault(); closeDialog(); }
  if (event.key === 'Tab') {
    const controls = Array.from(dialogElement.value?.querySelectorAll<HTMLElement>('input:not([disabled]),button:not([disabled])') || []), first = controls[0], last = controls[controls.length - 1];
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  }
}
function describeError(error: unknown, action: string) {
  const detail = (error instanceof Error ? error.message : String(error)).replace(/\s+/g, ' ').slice(0, 200);
  return `${action}${tr('失败，请重试', ' failed. Try again')}${detail ? ` · ${detail}` : ''}`;
}
async function saveDialog() {
  const target = dialog.value, generation = scopeGeneration; if (!target || saving.value || !props.items.some(item => item.id === target.id)) return;
  const title = titleDraft.value.trim(); if (target.kind === 'rename' && (!title || new TextEncoder().encode(title).length > 256)) { notice.value = tr('请输入标题，最多 256 字节。', 'Enter a title of at most 256 bytes.'); return; }
  if (target.kind === 'delete' && props.deletingDisabled) return;
  saving.value = true; notice.value = '';
  try {
    if (target.kind === 'rename') await props.updateConversation(target.id, { title });
    else if (!await props.removeConversation(target.id)) throw Error(tr('删除未完成', 'Deletion did not complete'));
    if (!disposed && generation === scopeGeneration && dialog.value?.id === target.id) { saving.value = false; closeDialog(); notice.value = target.kind === 'rename' ? tr('标题已保存', 'Title saved') : tr('对话已删除，工作区文件保留', 'Chat deleted. Workspace files retained'); }
  } catch (error) { if (!disposed && generation === scopeGeneration && dialog.value?.id === target.id) notice.value = describeError(error, target.kind === 'rename' ? tr('重命名', 'Rename') : tr('删除', 'Delete')); }
  finally { if (generation === scopeGeneration) saving.value = false; }
}
async function togglePin() {
  const item = menuItem.value, generation = scopeGeneration; if (!item || saving.value) return;
  saving.value = true; notice.value = '';
  try { await props.updateConversation(item.id, { pinned: !item.pinned }); if (!disposed && generation === scopeGeneration) { closeMenu(true); notice.value = item.pinned ? tr('已取消置顶', 'Unpinned') : tr('已置顶', 'Pinned'); } }
  catch (error) { if (!disposed && generation === scopeGeneration) notice.value = describeError(error, tr('置顶', 'Pin')); }
  finally { if (generation === scopeGeneration) saving.value = false; }
}
async function reveal() {
  const item = menuItem.value, generation = scopeGeneration; if (!item || saving.value) return;
  saving.value = true; notice.value = '';
  try { const path = await props.revealConversation(item.id); if (!disposed && generation === scopeGeneration) { closeMenu(true); notice.value = `${tr('已在 Finder 中显示项目共享工作区', 'Project shared workspace revealed')} · ${path}`; } }
  catch (error) { if (!disposed && generation === scopeGeneration) notice.value = describeError(error, tr('显示文件夹', 'Reveal folder')); }
  finally { if (generation === scopeGeneration) saving.value = false; }
}
function outside(event: Event) { if (menu.value && event.target instanceof Node && !menuElement.value?.contains(event.target)) closeMenu(); }
function dismissOnMove(event: Event) { if (menu.value && event.target instanceof Node && !menuElement.value?.contains(event.target)) closeMenu(); }
function resize() { closeMenu(); }
watch(() => props.projectId, () => { ++scopeGeneration; menu.value = undefined; dialog.value = undefined; saving.value = false; notice.value = ''; });
watch(() => props.items.map(item => item.id).join('\0'), () => { if (menu.value && !menuItem.value) closeMenu(); });
if (typeof window !== 'undefined') { window.addEventListener('pointerdown', outside, true); window.addEventListener('scroll', dismissOnMove, true); window.addEventListener('resize', resize); }
onBeforeUnmount(() => { disposed = true; ++scopeGeneration; if (typeof window !== 'undefined') { window.removeEventListener('pointerdown', outside, true); window.removeEventListener('scroll', dismissOnMove, true); window.removeEventListener('resize', resize); } });
</script>

<template>
  <div class="chat-history-list" data-testid="conversation-history-list">
    <section v-for="group in groups" :key="group.label" class="chat-history-group" :aria-label="group.label">
      <h3>{{group.label}}</h3>
      <div v-for="item in group.items" :key="item.id" class="chat-history-row" :class="{selected:item.id===selectedId}" @contextmenu="openMenu($event,item)" @keydown="rowKey($event,item)">
        <button type="button" class="chat-history-select" :title="label(item)" :aria-current="item.id===selectedId?'page':undefined" @click="emit('select',item.id)"><Pin v-if="item.pinned" :size="14"/><MessageSquare v-else :size="14"/><span><strong>{{label(item)}}</strong><time v-if="time(item).short" :datetime="conversationActivityAt(item)" :title="tr('最近活动：','Last activity: ')+time(item).full">{{time(item).short}}</time></span></button>
        <button type="button" class="chat-history-more" :disabled="loading||saving" :aria-label="tr(`会话操作：${label(item)}`,`Chat actions: ${label(item)}`)" :aria-expanded="menu?.id===item.id" aria-haspopup="menu" @click="openMenu($event,item)"><Ellipsis :size="17"/></button>
      </div>
    </section>
    <p v-if="!loading&&!items.length" class="chat-history-empty">{{emptyLabel}}</p>
    <p v-if="notice&&!dialog" class="chat-history-notice" role="status">{{notice}}</p>
  </div>
  <Teleport to="body">
    <div v-if="menu&&menuItem" ref="menuElement" class="chat-history-menu" role="menu" :aria-label="tr('会话操作','Chat actions')" :style="{left:`${menu.x}px`,top:`${menu.y}px`}" @keydown="menuKey">
      <button type="button" role="menuitem" :disabled="saving" @click="openDialog('rename')"><Pencil :size="15"/>{{tr('重命名','Rename')}}</button>
      <button type="button" role="menuitem" :disabled="saving" @click="togglePin"><PinOff v-if="menuItem.pinned" :size="15"/><Pin v-else :size="15"/>{{menuItem.pinned?tr('取消置顶','Unpin'):tr('置顶','Pin')}}</button>
      <button type="button" role="menuitem" :disabled="saving" :title="tr('显示此项目共享的 Agent 工作区','Reveal the Agent workspace shared by this project')" @click="reveal"><FolderOpen :size="15"/>{{saving?tr('正在处理…','Working…'):tr('在 Finder 中显示工作区','Reveal workspace in Finder')}}</button>
      <button type="button" role="menuitem" class="danger" :disabled="saving||deletingDisabled" @click="openDialog('delete')"><Trash2 :size="15"/>{{tr('删除对话…','Delete chat…')}}</button>
    </div>
    <div v-if="dialog" class="chat-history-dialog-backdrop" @click.self="closeDialog">
      <form ref="dialogElement" class="chat-history-dialog" role="dialog" aria-modal="true" :aria-label="dialog.kind==='rename'?tr('重命名会话','Rename chat'):tr('删除会话','Delete chat')" @keydown="dialogKey" @submit.prevent="saveDialog">
        <header><strong>{{dialog.kind==='rename'?tr('重命名会话','Rename chat'):tr('删除会话','Delete chat')}}</strong><button type="button" :disabled="saving" :aria-label="tr('关闭','Close')" @click="closeDialog"><X :size="17"/></button></header>
        <input v-if="dialog.kind==='rename'" ref="renameInput" v-model="titleDraft" :disabled="saving" :aria-label="tr('会话标题','Chat title')" autocomplete="off"/>
        <p v-else>{{tr(`删除“${dialog.title}”的聊天记录？项目资产和工作区文件会保留。`,`Delete the chat history for “${dialog.title}”? Project assets and workspace files will be retained.`)}}</p>
        <p v-if="notice" class="chat-history-dialog-notice" role="status">{{notice}}</p>
        <footer><button type="button" :disabled="saving" @click="closeDialog">{{tr('取消','Cancel')}}</button><button type="submit" :disabled="saving||(dialog.kind==='rename'&&!titleDraft.trim())||(dialog.kind==='delete'&&deletingDisabled)" :class="{danger:dialog.kind==='delete',primary:dialog.kind==='rename'}">{{saving?tr('保存中…','Saving…'):dialog.kind==='rename'?tr('保存标题','Save title'):tr('删除对话','Delete chat')}}</button></footer>
      </form>
    </div>
  </Teleport>
</template>

<style scoped>
.chat-history-list{flex:1;min-height:0;overflow:auto;scrollbar-width:thin;overscroll-behavior:contain}.chat-history-group h3{margin:9px 10px 5px;color:var(--app-muted,#72746f);font-size:10px;font-weight:500}.chat-history-row{display:flex;align-items:center;min-width:0;margin-bottom:3px;border-radius:8px}.chat-history-row.selected,.chat-history-row:hover{background:color-mix(in srgb,var(--app-ink,#252624) 7%,transparent)}.chat-history-select{display:flex;gap:8px;align-items:flex-start;flex:1;min-width:0;padding:9px 6px 9px 10px;border:0;background:transparent;text-align:left;color:var(--app-ink,#252624);font:inherit;cursor:pointer}.chat-history-select>svg{flex-shrink:0;margin-top:3px}.chat-history-select>span{display:flex;flex-direction:column;gap:3px;min-width:0}.chat-history-select strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12px;font-weight:500;line-height:1.5}.chat-history-select time{font-size:10px;line-height:1.3;color:var(--app-muted,#72746f)}.chat-history-more{display:grid;place-items:center;flex-shrink:0;width:28px;height:28px;margin-right:4px;padding:0;border:0;border-radius:6px;background:transparent;color:var(--app-muted,#72746f);cursor:pointer}.chat-history-more:hover,.chat-history-more[aria-expanded=true]{background:color-mix(in srgb,var(--app-ink,#252624) 8%,transparent);color:var(--app-ink,#252624)}.chat-history-more:disabled{opacity:.4;cursor:default}.chat-history-empty,.chat-history-notice{user-select:text;margin:8px 10px;color:var(--app-muted,#72746f);font-size:11px;line-height:1.5;overflow-wrap:anywhere}
.chat-history-menu{position:fixed;z-index:1400;display:flex;flex-direction:column;width:224px;max-width:calc(100vw - 16px);max-height:calc(100dvh - 16px);overflow:auto;padding:5px;border:1px solid var(--line,#ddd);border-radius:10px;background:var(--app-surface,#fff);box-shadow:0 10px 32px #0002;color:var(--app-ink,#252624)}.chat-history-menu button{display:flex;align-items:center;gap:9px;padding:10px;border:0;border-radius:6px;background:transparent;text-align:left;color:inherit;font:inherit;font-size:12px;cursor:pointer}.chat-history-menu button:hover,.chat-history-menu button:focus-visible{background:color-mix(in srgb,var(--blue,#2878ff) 9%,transparent);outline:0}.chat-history-menu .danger{margin-top:4px;border-top:1px solid var(--line,#ddd);border-radius:0 0 6px 6px;color:var(--red,#b63f3f)}.chat-history-menu button:disabled{opacity:.4;cursor:default}
.chat-history-dialog-backdrop{position:fixed;inset:0;z-index:1450;display:flex;align-items:center;justify-content:center;padding:16px;background:#0005;color:var(--app-ink,#252624)}.chat-history-dialog{box-sizing:border-box;width:min(440px,100%);max-height:calc(100dvh - 32px);overflow:auto;padding:18px;border:1px solid var(--line,#ddd);border-radius:14px;background:var(--app-surface,#fff);box-shadow:0 20px 60px #0003}.chat-history-dialog header{display:flex;align-items:center;justify-content:space-between;margin-bottom:16px}.chat-history-dialog header strong{font-size:15px}.chat-history-dialog header button{display:grid;place-items:center;width:26px;height:26px;padding:0;border:0;background:transparent;color:var(--app-muted,#72746f)}.chat-history-dialog input{box-sizing:border-box;width:100%;padding:10px;border:1px solid var(--line,#ddd);border-radius:8px;background:transparent;color:inherit;font:inherit;font-size:13px}.chat-history-dialog p{font-size:13px;line-height:1.7;overflow-wrap:anywhere}.chat-history-dialog .chat-history-dialog-notice{color:var(--red,#b63f3f);font-size:12px}.chat-history-dialog footer{display:flex;justify-content:flex-end;gap:8px;margin-top:18px}.chat-history-dialog footer button{padding:8px 12px;border:1px solid var(--line,#ddd);border-radius:7px;background:transparent;color:inherit;font:inherit;font-size:12px;cursor:pointer}.chat-history-dialog footer .primary{border-color:var(--blue,#2878ff);background:var(--blue,#2878ff);color:#fff}.chat-history-dialog footer .danger{border-color:var(--red,#b63f3f);background:var(--red,#b63f3f);color:#fff}.chat-history-dialog button:disabled{opacity:.5;cursor:default}
</style>
