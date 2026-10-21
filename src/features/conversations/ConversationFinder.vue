<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, useId, watch } from 'vue';
import { MessageSquare, Search, X } from '@lucide/vue';
import { useI18n } from '../../i18n';
const props=defineProps<{open:boolean;query:string;items:Array<{id:string;title:string;createdAt?:string;archived?:boolean}>;selectedId?:string;projectName?:string;loading?:boolean;hasMore?:boolean;disabled?:boolean}>();
const emit=defineEmits<{'update:open':[value:boolean];'update:query':[value:string];select:[id:string];more:[]}>();
const {tr}=useI18n();
const input=ref<HTMLInputElement>(),panel=ref<HTMLElement>();
const active=ref(0),limit=ref(60),listId=`chat-finder-${useId()}`;
let previousFocus:HTMLElement|undefined,generation=0;
const results=computed(()=>props.items.filter(item=>item.title.toLocaleLowerCase().includes(props.query.trim().toLocaleLowerCase())));
const visible=computed(()=>results.value.slice(0,limit.value));
function close(){emit('update:open',false);}
function choose(id:string){if(props.disabled)return;emit('select',id);close();}
function more(){if(limit.value<results.value.length)limit.value+=60;else emit('more');}
function handleKey(event:KeyboardEvent){
  if(event.isComposing)return;
  if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close();return;}
  if(['ArrowDown','ArrowUp'].includes(event.key)){
    event.preventDefault();const count=visible.value.length;if(!count)return;
    active.value=(active.value+(event.key==='ArrowDown'?1:-1)+count)%count;
    void nextTick(()=>panel.value?.querySelector(`[data-result-index="${active.value}"]`)?.scrollIntoView({block:'nearest'}));
  }
  if(event.key==='Enter'&&event.target===input.value){event.preventDefault();const item=visible.value[active.value];if(item)choose(item.id);}
  if(event.key==='Tab'){
    const controls=Array.from(panel.value?.querySelectorAll<HTMLElement>('button:not([disabled]),input')||[]);
    const first=controls[0],last=controls[controls.length-1];
    if(event.shiftKey&&document.activeElement===first){event.preventDefault();last?.focus();}
    else if(!event.shiftKey&&document.activeElement===last){event.preventDefault();first?.focus();}
  }
}
watch(()=>props.query,()=>{active.value=0;limit.value=60;});
watch(()=>props.items,()=>{active.value=Math.min(active.value,Math.max(0,visible.value.length-1));});
watch(()=>props.open,async open=>{
  const current=++generation;
  if(open){previousFocus=typeof document!=='undefined'&&document.activeElement&&'focus' in document.activeElement?document.activeElement as HTMLElement:undefined;active.value=0;limit.value=60;await nextTick();if(current===generation&&props.open){input.value?.focus?.();input.value?.select?.();}}
  else{await nextTick();if(current===generation&&previousFocus?.isConnected)previousFocus.focus();}
},{immediate:true});
onBeforeUnmount(()=>{++generation;});
</script>
<template>
  <Teleport defer to=".app-shell">
    <div v-if="open" class="conversation-finder-backdrop" @click.self="close">
      <section ref="panel" class="conversation-finder" role="dialog" aria-modal="true" :aria-label="tr('查找对话','Find a conversation')" @keydown="handleKey">
        <header><Search :size="19"/><input ref="input" :value="query" :aria-label="tr('搜索完整对话标题','Search full conversation titles')" :placeholder="tr('输入标题、域名或关键词…','Search titles, domains or keywords…')" role="combobox" :aria-controls="listId" aria-expanded="true" :aria-activedescendant="visible.length?`${listId}-${active}`:undefined" autocomplete="off" @input="emit('update:query',($event.target as HTMLInputElement).value)"/><button type="button" :aria-label="tr('关闭查找','Close finder')" @click="close"><X :size="18"/></button></header>
        <div class="conversation-finder-context"><span>{{projectName||tr('当前工作空间','Current workspace')}}</span><span>{{loading?tr('读取中…','Loading…'):tr(`${results.length} 个匹配`,`${results.length} matches`)}}</span></div>
        <div :id="listId" class="conversation-finder-results" role="listbox" :aria-label="tr('匹配的对话','Matching conversations')">
          <button v-for="(item,index) in visible" :id="`${listId}-${index}`" :data-result-index="index" :key="item.id" type="button" role="option" :aria-selected="item.id===selectedId" :disabled="disabled" :class="{highlighted:index===active}" @mouseenter="active=index" @click="choose(item.id)"><MessageSquare :size="17"/><span><strong>{{item.title}}</strong><small v-if="item.archived">{{tr('已归档','Archived')}}</small></span><small v-if="item.id===selectedId" class="conversation-finder-current">{{tr('当前','Current')}}</small></button>
          <p v-if="!loading&&!visible.length">{{tr('没有匹配的对话，试试标题中的域名或关键词。','No matching conversations. Try a domain or a word from the title.')}}</p>
          <button v-if="limit<results.length||hasMore" type="button" class="conversation-finder-more" :disabled="loading" @click="more">{{tr('显示更多对话','Show more conversations')}}</button>
        </div>
        <footer><span>{{tr('↑ ↓ 选择 · Enter 打开 · Esc 关闭','↑ ↓ select · Enter open · Esc close')}}</span><span>{{tr('只查本地会话','Local conversations')}}</span></footer>
      </section>
    </div>
  </Teleport>
</template>
<style scoped>
.conversation-finder-backdrop{position:fixed;inset:0;z-index:1100;background:#0005;display:flex;align-items:flex-start;justify-content:center;padding:clamp(24px,12vh,110px) 20px 24px;color:var(--app-ink,#252624)}
.conversation-finder{display:flex;flex-direction:column;width:min(720px,100%);max-height:calc(100dvh - 150px);min-height:180px;background:var(--app-surface,#fff);border:1px solid var(--line,#ddd);border-radius:16px;box-shadow:0 24px 80px #0003;overflow:hidden}
.conversation-finder header{display:flex;align-items:center;gap:12px;padding:16px 18px;border-bottom:1px solid var(--line)}.conversation-finder header input{min-width:0;flex:1;border:0;outline:0;background:transparent;color:inherit;font:inherit;font-size:15px;padding:6px 0;box-shadow:none}.conversation-finder header>button{border:0;background:transparent;color:var(--app-muted,#72746f);display:grid;place-items:center;width:30px;height:30px;border-radius:7px}
.conversation-finder-context{display:flex;justify-content:space-between;gap:12px;padding:12px 18px 6px;color:var(--app-muted,#72746f);font-size:11px}.conversation-finder-context>span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.conversation-finder-results{min-height:0;overflow:auto;overscroll-behavior:contain;padding:6px 10px 12px;scrollbar-width:thin}.conversation-finder-results>button{display:flex;align-items:flex-start;gap:12px;width:100%;border:0;border-radius:9px;padding:13px 12px;background:transparent;color:inherit;text-align:left;cursor:pointer}.conversation-finder-results>button.highlighted{background:color-mix(in srgb,var(--blue,#2878ff) 9%,transparent)}.conversation-finder-results>button>svg{flex-shrink:0;margin-top:2px;color:var(--app-muted,#72746f)}.conversation-finder-results>button>span{flex:1;min-width:0;display:flex;flex-direction:column;gap:5px}.conversation-finder-results strong{font-size:13px;font-weight:500;line-height:1.6;white-space:normal;overflow-wrap:anywhere}.conversation-finder-results small{font-size:10px;color:var(--app-muted,#72746f)}.conversation-finder-current{flex-shrink:0;padding-top:4px}.conversation-finder-results>p{padding:25px 12px;font-size:13px;color:var(--app-muted,#72746f)}.conversation-finder-results .conversation-finder-more{justify-content:center;color:var(--blue)}
.conversation-finder footer{display:flex;justify-content:space-between;gap:12px;padding:12px 18px;border-top:1px solid var(--line);color:var(--app-muted,#72746f);font-size:10px}
@media(max-width:760px){.conversation-finder-backdrop{padding:20px 12px}.conversation-finder{max-height:calc(100dvh - 40px)}.conversation-finder footer>span:last-child{display:none}}
</style>
