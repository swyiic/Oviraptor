<script setup lang="ts">
import {ref,computed,onMounted} from 'vue';
import {invoke} from '@tauri-apps/api/core';
import {open} from '@tauri-apps/plugin-dialog';
import {knowledgeCatalogApi} from './catalogApi';
import {skillsApi} from './api';
import type {AgentKnowledgeEntry} from './knowledgeTypes';
import type {SkillPackage} from './types';
import {useI18n} from '../../i18n';
const {tr}=useI18n();
const source=ref(''),content=ref(''),busy=ref(false),error=ref(''),notice=ref(''),search=ref('');
const entries=ref<AgentKnowledgeEntry[]>([]),packages=ref<SkillPackage[]>([]),autoAssign=ref(true),targets=ref<Record<number,string>>({});
const visible=computed(()=>entries.value.filter(e=>!e.projectId&&`${e.title} ${e.summary} ${JSON.stringify(e.patterns.methodCards||[])}`.toLowerCase().includes(search.value.trim().toLowerCase())));
async function load(){
 [entries.value,packages.value]=await Promise.all([knowledgeCatalogApi.listAgentKnowledge(),skillsApi.listPackages()]);
 for(const entry of entries.value) if(targets.value[entry.id]===undefined) targets.value[entry.id]='';
}
onMounted(()=>load().catch(e=>error.value=String(e)));
async function assign(id:number){return invoke('assign_agent_knowledge_package',{knowledgeId:id,targetSkillId:targets.value[id]?Number(targets.value[id]):null});}
async function distill(){if(busy.value)return;busy.value=true;error.value='';notice.value='';try{
 const entry=await invoke<AgentKnowledgeEntry>('ingest_agent_knowledge_source',{source:source.value,content:content.value||null,forceRefresh:false});
 notice.value=tr('知识已保存并按正文去重。','Knowledge saved and deduplicated by content.');await load();
 if(autoAssign.value&&!entry.skillId){await assign(entry.id);notice.value=tr('蒸馏完成，已归入技能包的知识资源。','Distillation complete. Added to the knowledge resources of a skill package.');await load();}
}catch(e){error.value=String(e)}finally{busy.value=false}}
async function attach(id:number){busy.value=true;error.value='';try{await assign(id);await load();notice.value=tr('知识已归入技能包。','Knowledge added to the skill package.')}catch(e){error.value=String(e)}finally{busy.value=false}}
async function pick(){const p=await open({multiple:false,filters:[{name:tr('技术资料','Technical documents'),extensions:['md','txt','html','htm']}]});if(typeof p==='string')source.value=p;}
</script>
<template>
<section class="distillation">
 <header><div><p>{{ tr('技术文章 → AI 提炼 → 知识归档 → 技能包复用。技能包统一在设置中管理。', 'Technical article → AI distillation → knowledge archive → reuse in skill packages. Skill packages are managed in Settings.') }}</p></div><span>{{ tr(`${entries.filter(e=>!e.projectId).length} 条知识`, `${entries.filter(e=>!e.projectId).length} knowledge entries`) }}</span></header>
 <div class="distill-source"><h3>{{ tr('添加技术资料', 'Add technical material') }}</h3><label>{{ tr('文章链接或本地文件', 'Article URL or local file') }}<div class="source-row"><input v-model="source" :disabled="busy" :placeholder="tr('https://… 或 Markdown / HTML / TXT 路径', 'https://… or a Markdown / HTML / TXT path')"/><button class="button ghost compact" :disabled="busy" @click="pick">{{ tr('选择文件', 'Choose file') }}</button></div></label>
 <label>{{ tr('或粘贴正文', 'Or paste the text') }}<textarea v-model="content" :disabled="busy" rows="7" :placeholder="tr('编程、架构、调试、运维、安全等技术文章；粘贴正文时优先分析正文。', 'Technical articles on programming, architecture, debugging, operations, security, and more. Pasted text takes priority.')"/></label>
 <footer><label class="check"><input v-model="autoAssign" type="checkbox" :disabled="busy"/>{{ tr('蒸馏后由 AI 选择合适的技能包', 'Let AI pick a suitable skill package after distillation') }}</label><button class="button primary compact" :disabled="busy||(!source.trim()&&!content.trim())" @click="distill">{{busy?tr('正在处理…', 'Processing…'):tr('开始蒸馏', 'Start distillation')}}</button></footer>
 <small>{{ tr('支持可读取的网页和文本资料，单篇最多 8 万字符；PDF、图片请先转换为正文。归类失败时仍保留已蒸馏知识。', 'Supports readable web pages and text, up to 80,000 characters each. Convert PDFs and images to text first. Distilled knowledge is kept even if categorization fails.') }}</small></div>
 <p v-if="error" role="alert" class="error">{{error}}</p><p v-if="notice" role="status">{{notice}}</p>
 <div class="library-head"><h3>{{ tr('已沉淀知识', 'Saved knowledge') }}</h3><input v-model="search" type="search" :placeholder="tr('搜索标题或知识内容', 'Search titles or knowledge content')" :aria-label="tr('搜索知识', 'Search knowledge')"/></div>
 <p v-if="!visible.length" class="empty">{{ tr('暂无匹配的知识，添加一篇技术文章开始。', 'No matching knowledge. Add a technical article to get started.') }}</p>
 <article v-for="entry in visible" :key="entry.id"><header><h3>{{entry.title}}</h3><span>{{entry.skillId ? (packages.find(p=>p.skills.some(s=>s.id===entry.skillId))?.name || tr('已归入技能', 'Assigned to a skill')) : tr('待归类', 'Unassigned')}}</span></header><p>{{entry.summary}}</p>
 <p v-if="entry.patterns.packageAssignment" class="assignment">{{(entry.patterns.packageAssignment as any).reason}}</p>
 <details><summary>{{ tr('查看知识与来源', 'View knowledge and source') }}</summary><pre>{{JSON.stringify(entry.patterns.methodCards||entry.patterns,null,2)}}</pre><small>{{entry.patterns.sourceUrl}}</small><details v-if="entry.patterns.originalText"><summary>{{ tr('保留的原文', 'Preserved original text') }}</summary><pre>{{entry.patterns.originalText}}</pre></details></details>
 <footer><select v-model="targets[entry.id]" :disabled="busy" :aria-label="tr('归属技能包', 'Target skill package')"><option value="">{{ tr('AI 自动判断归属', 'Let AI decide') }}</option><optgroup v-for="pack in packages" :key="pack.id" :label="pack.name"><option v-for="skill in pack.skills" :key="skill.id" :value="String(skill.id)">{{skill.name}}</option></optgroup></select><button class="button ghost compact" :disabled="busy" @click="attach(entry.id)">{{entry.skillId?tr('调整归属', 'Change assignment'):tr('归入技能包', 'Add to skill package')}}</button></footer></article>
</section>
</template>
<style scoped>
.distillation{min-width:0;display:flex;flex-direction:column;gap:16px;color:var(--ui-ink)}header,footer,.library-head,.source-row{display:flex;align-items:center;justify-content:space-between;gap:12px}h2,h3,p{margin:0}h2{font-size:19px}h3{font-size:14px}p,small,summary,label,span{font-size:12px;line-height:1.7}header p,small,.empty{color:var(--ui-muted)}.distill-source,article{padding:16px;border:1px solid var(--ui-line);border-radius:12px;background:var(--ui-surface);display:flex;flex-direction:column;gap:12px}label{display:flex;flex-direction:column;gap:6px}input:not([type=checkbox]),select,textarea{min-width:0;width:100%;border:1px solid var(--ui-line);background:var(--ui-surface);color:inherit;border-radius:8px;padding:9px 10px;font:inherit;font-size:13px;box-sizing:border-box}textarea{resize:vertical}.source-row input{flex:1}.button{white-space:nowrap;height:36px;min-height:36px;font-size:12px}.check{flex-direction:row;align-items:center}footer select{max-width:420px}.library-head input{max-width:320px}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:300px;overflow:auto;font:12px/1.6 var(--font-mono);background:var(--ui-surface-raised);padding:12px;border-radius:8px}summary{cursor:pointer}.error{color:var(--red);overflow-wrap:anywhere}.assignment{color:var(--ui-muted)}@media(max-width:800px){header,footer,.library-head{align-items:stretch;flex-direction:column}.library-head input,footer select{max-width:none}}
</style>
