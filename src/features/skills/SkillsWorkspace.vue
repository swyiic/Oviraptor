<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../shared/directives/refreshFeedback';
import { computed, nextTick, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { BookOpen, Braces, ChevronDown, ChevronRight, Download, FileText, FolderOpen, Info, Package, RefreshCw, Search, ShieldAlert, X } from "@lucide/vue";
import { useI18n } from "../../i18n";
import SkillInstructionsCatalog from "./SkillInstructionsCatalog.vue";
import { useSkillsCatalog } from "./useSkillsCatalog";
import { skillsApi } from "./api";
import { useSkillsLibrary } from "./useSkillsLibrary";
import type { PackagedSkill, SkillPackage } from "./types";
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();
const { tr } = useI18n();
const library = useSkillsLibrary({ api: skillsApi, pickDirectory: () => open({ multiple: false, directory: true, title: tr("选择包含 SKILL.md 或 skills/ 的目录", "Choose a folder containing SKILL.md or skills/") }) });
const { packages, loading, error, search, preview, previewBusy, importing, installing, changing, selected, resources, resourcesLoading, resourceError, selectedResource, resourcePage, resourceText, resourceLoading, packageSkillIds, visiblePackages } = library;
const textCatalog = useSkillsCatalog({ tr, notify: (type, text) => emit("notify", type, text), clearSelection: () => {}, context: "library" });
const totalSkills = computed(() => packages.value.reduce((sum, item) => sum + item.skills.length, 0));
const enabledSkills = computed(() => packages.value.reduce((sum, item) => sum + item.skills.filter(skill => skill.enabled).length, 0));
const expandedPackages = ref(new Set<string>());
const resourcePanel = ref<HTMLElement>();
function togglePackage(id: string) {
  const next = new Set(expandedPackages.value);
  if (next.has(id)) next.delete(id); else next.add(id);
  expandedPackages.value = next;
}
function ownerOf(item: SkillPackage) { return packages.value.find(owner => owner.id === item.id) || item; }
async function openSkill(item: SkillPackage, skill: PackagedSkill) {
  const reading = library.openSkill(ownerOf(item), skill);
  await nextTick();
  if (selected.value?.skill.id === skill.id && typeof window !== "undefined" && window.matchMedia("(max-width: 1100px)").matches) {
    resourcePanel.value?.scrollIntoView({ block: "nearest" });
  }
  await reading;
}
function packageEnabled(item: SkillPackage) { return ownerOf(item).skills.filter(skill => skill.enabled).length; }
function packageDescription(item: SkillPackage) {
  const owner = ownerOf(item);
  if (owner.skills.length === 1) return owner.skills[0].description;
  if (/compound engineering/i.test(owner.name)) return tr('规划、开发、评审与复盘。按需展开使用包内技能。', 'Planning, development, review, and learning. Expand to use the skills in this package.');
  return tr(`包含 ${owner.skills.length} 项技能，可按需查看指令与参考资源。`, `${owner.skills.length} skills with instructions and reference resources.`);
}
function bytes(value: number) { return value < 1024 ? `${value} B` : value < 1024 * 1024 ? `${(value / 1024).toFixed(1)} KB` : `${(value / (1024 * 1024)).toFixed(1)} MB`; }
function toggle(skill: PackagedSkill, event: Event) { const input = event.target as HTMLInputElement; const enabled = input.checked; input.checked = skill.enabled; void library.setEnabled(skill, enabled); }
</script>
<template>
  <section class="skills-workspace" :aria-label="tr('独立 Skills 管理','Independent Skills manager')">
    <header class="skills-library-header">
      <div class="skills-library-intro"><h2>{{tr('技能库','Skill library')}}</h2><p>{{tr('按技能包管理，在对话中选择需要的技能。','Manage packages and choose the skills needed in each conversation.')}}</p></div>
      <div class="skills-header-actions"><button v-refresh-feedback="{busy:loading,error}" class="button ghost" :disabled="loading" :aria-label="tr('刷新 Skills','Refresh Skills')" @click="library.loadPackages()"><RefreshCw :size="16"/></button><button class="button primary" :disabled="previewBusy||importing" @click="library.choosePackage()"><FolderOpen :size="16"/>{{previewBusy?tr('读取中…','Reading…'):tr('导入本地技能包','Import local package')}}</button></div>
    </header>
    <p v-if="error" class="skills-error" role="alert">{{error}}</p>
    <section v-if="preview" class="skills-import-preview" :aria-label="tr('导入预览','Import preview')">
      <header><div><span class="skills-eyebrow">{{tr('导入预览','IMPORT PREVIEW')}}</span><h3>{{preview.name}}</h3></div><button class="icon-button subtle" :disabled="importing" :aria-label="tr('取消导入','Cancel import')" @click="library.cancelPreview()"><X :size="18"/></button></header>
      <p>{{tr(`${preview.skillCount} 个技能 · ${preview.fileCount} 个文件 · ${bytes(preview.totalBytes)}`,`${preview.skillCount} skills · ${preview.fileCount} files · ${bytes(preview.totalBytes)}`)}}</p>
      <ul class="skills-preview-entries"><li v-for="skill in preview.skills" :key="skill.entryPath"><details><summary><strong>{{skill.name}}</strong><span class="skills-description-clamp">{{skill.description}}</span></summary><p>{{skill.description}}</p><code>{{skill.entryPath}}</code></details></li></ul>
      <div v-if="preview.warnings.length||preview.missingCapabilities.length" class="skills-capability-notice"><ShieldAlert :size="16"/><div><strong>{{tr('导入前请查看','Review before importing')}}</strong><ul><li v-for="warning in preview.warnings" :key="warning">{{warning}}</li><li v-for="capability in preview.missingCapabilities" :key="capability">{{tr('尚不支持的能力：','Unavailable capability: ')}}{{capability}}</li></ul></div></div>
      <p class="skills-local-notice">{{tr('导入仅保存本地文件，不执行脚本，也不会授予目录、网络或工具权限。文件变化后需要重新预览。','Import saves local files only. It does not run scripts or grant directory, network, or tool access. Changed files require a new preview.')}}</p>
      <footer><span :title="preview.digest">SHA-256 · {{preview.digest.slice(0,12)}}</span><button class="button ghost" :disabled="importing" @click="library.cancelPreview()">{{tr('取消','Cancel')}}</button><button class="button primary" :disabled="importing" @click="library.importPreview()"><Download :size="15"/>{{importing?tr('导入中…','Importing…'):tr('确认导入','Import package')}}</button></footer>
    </section>
    <div class="skills-library-toolbar"><label class="skills-search"><Search :size="16"/><input v-model="search" type="search" :aria-label="tr('搜索技能库','Search skills library')" :placeholder="tr('搜索技能、用途或来源…','Search skills, purpose, or source…')"/></label><span class="skills-library-count">{{tr(`${packages.length} 个技能包 · ${totalSkills} 项技能 · ${enabledSkills} 项已启用`,`${packages.length} packages · ${totalSkills} skills · ${enabledSkills} enabled`)}}</span></div>
    <p v-if="loading" class="skills-muted" role="status">{{tr('正在读取技能库…','Loading skills library…')}}</p>
    <div class="skills-library-layout" :class="{'with-resource':selected}">
      <div class="skills-package-list">
        <article v-for="item in visiblePackages" :key="item.id" class="skills-package" :class="{'is-expanded':expandedPackages.has(item.id)}" :data-package-id="item.id">
          <header class="skills-package-heading">
            <span class="skills-package-icon"><Package :size="20"/></span>
            <div class="skills-package-main">
              <div class="skills-package-title"><h3>{{item.name}}</h3><span class="skills-source-badge">{{item.builtin?tr('内置','Bundled'):tr('已导入','Imported')}}</span></div>
              <p class="skills-package-description skills-description-clamp">{{packageDescription(item)}}</p>
              <div class="skills-package-meta"><span>{{tr(`${ownerOf(item).skills.length} 项技能`,`${ownerOf(item).skills.length} skills`)}}</span><span>{{tr(`${packageEnabled(item)} 项已启用`,`${packageEnabled(item)} enabled`)}}</span><span v-if="search.trim()" class="skills-match-count">{{tr(`${item.skills.length} 项匹配`,`${item.skills.length} matches`)}}</span></div>
            </div>
            <button type="button" class="button ghost skills-package-expand" :aria-expanded="expandedPackages.has(item.id)" :aria-controls="`skills-members-${item.id}`" @click="togglePackage(item.id)"><span>{{expandedPackages.has(item.id)?tr('收起','Collapse'):tr('查看技能','View skills')}}</span><ChevronDown v-if="expandedPackages.has(item.id)" :size="15"/><ChevronRight v-else :size="15"/></button>
          </header>
          <details class="skills-package-info">
            <summary><Info :size="14"/><span>{{tr('来源与能力说明','Source & capabilities')}}</span><ChevronDown :size="13"/></summary>
            <dl><div><dt>{{tr('来源','Source')}}</dt><dd>{{item.source||tr('未标注','Unspecified')}}</dd></div><div><dt>{{tr('版本','Version')}}</dt><dd>{{item.version||tr('未标注','Unspecified')}}</dd></div><div><dt>{{tr('许可','License')}}</dt><dd>{{item.license||tr('未标注','Unspecified')}}</dd></div><div><dt>{{tr('资源','Resources')}}</dt><dd>{{tr(`${item.fileCount} 个文件 · ${bytes(item.totalBytes)}`,`${item.fileCount} files · ${bytes(item.totalBytes)}`)}}</dd></div></dl>
            <div v-if="item.missingCapabilities.length" class="skills-capability-note"><strong>{{tr('部分步骤需要额外能力','Some steps need additional capabilities')}}</strong><p>{{tr('技能包提供指令与参考；实际可用工具由当前会话决定。','Packages supply instructions and references; available tools depend on the current conversation.')}}</p><ul><li v-for="capability in item.missingCapabilities" :key="capability">{{capability}}</li></ul></div>
          </details>
          <section v-if="expandedPackages.has(item.id)" :id="`skills-members-${item.id}`" class="skills-members" :aria-label="tr(`${item.name} 包内技能`,`${item.name} skills`)">
            <div class="skills-members-heading"><span>{{tr('包内技能','Package skills')}}</span><span>{{tr('选择技能查看完整说明与资源','Select a skill to read its full description and resources')}}</span></div>
            <div class="skills-member-list" tabindex="0" :aria-label="tr('可滚动的技能列表','Scrollable skill list')">
              <article v-for="skill in item.skills" :key="skill.id" class="skills-member" :class="{selected:selected?.skill.id===skill.id,disabled:!skill.enabled}">
                <button type="button" class="skills-member-open" :aria-label="tr(`查看 ${skill.name} 的指令与资源`,`View instructions and resources for ${skill.name}`)" :aria-pressed="selected?.skill.id===skill.id" @click="openSkill(item,skill)"><BookOpen :size="16"/><span><strong>{{skill.name}}</strong><span class="skills-description-clamp">{{skill.description||tr('查看指令与参考资源','View instructions and reference resources')}}</span></span><ChevronRight :size="15"/></button>
                <label class="skill-enabled-toggle"><input type="checkbox" :checked="skill.enabled" :disabled="changing.has(skill.id)" :aria-label="tr(`启用 ${skill.name}`,`Enable ${skill.name}`)" @change="toggle(skill,$event)"/><span>{{skill.enabled?tr('已启用','Enabled'):tr('已停用','Disabled')}}</span></label>
              </article>
            </div>
          </section>
        </article>
        <div v-if="!loading&&!visiblePackages.length" class="skills-empty"><Braces :size="32"/><h3>{{search.trim()?tr('没有匹配的技能','No matching skills'):tr('从一个技能包开始','Start with a skill package')}}</h3><p>{{tr('可选择单个 SKILL.md 所在目录，或包含 skills/ 的技能合集。压缩包请先解压。','Choose a single SKILL.md folder or a package containing skills/. Extract ZIP archives before importing.')}}</p></div>
        <section v-if="!loading&&!packages.some(item=>item.builtin)" class="skills-bundled-install"><div><strong>{{tr('内置 Compound 技能库','Bundled Compound skill library')}}</strong><p>{{tr('从随应用附带的本地资源安装，无需下载。','Install from the local resources included with the app. No download is needed.')}}</p></div><button class="button ghost" :disabled="installing" @click="library.installBundled()">{{installing?tr('安装中…','Installing…'):tr('添加内置技能','Add bundled skills')}}</button></section>
      </div>
      <aside v-if="selected" ref="resourcePanel" class="skills-resource-panel" :aria-label="tr('技能资源预览','Skill resource preview')">
        <header><div><span class="skills-eyebrow">{{tr('只读预览','READ-ONLY PREVIEW')}}</span><h3>{{selected.skill.name}}</h3></div><button class="icon-button subtle" :aria-label="tr('关闭资源预览','Close resource preview')" @click="library.closeResource()"><X :size="17"/></button></header>
        <p class="skills-resource-origin">{{selected.package.name}}</p>
        <details v-if="selected.skill.description" :key="selected.skill.id" class="skills-full-description"><summary>{{tr('完整技能说明','Full skill description')}}</summary><p>{{selected.skill.description}}</p></details>
        <p v-if="resourcesLoading" class="skills-muted" role="status">{{tr('正在读取资源列表…','Loading resource list…')}}</p>
        <nav class="skills-resource-files" :aria-label="tr('技能文件','Skill files')"><button v-for="resource in resources" :key="resource.path" :class="{active:selectedResource?.path===resource.path}" :aria-current="selectedResource?.path===resource.path?'true':undefined" @click="library.readResource(resource)"><FileText :size="14"/><span>{{resource.path}}</span><small>{{bytes(resource.sizeBytes)}}</small></button></nav>
        <p v-if="resourceError" class="skills-error" role="alert">{{resourceError}}</p>
        <template v-if="selectedResource"><div class="skills-file-heading"><strong>{{selectedResource.path}}</strong><span>{{bytes(selectedResource.sizeBytes)}}</span></div><p v-if="selectedResource.kind==='script'" class="skills-script-notice">{{tr('脚本仅供阅读，不会从此页面执行。','Scripts are read-only here and are never executed from this page.')}}</p><p v-if="resourceLoading" class="skills-muted" role="status">{{tr('正在读取文件…','Reading file…')}}</p><p v-if="resourcePage?.encoding==='binary'" class="skills-muted">{{tr('已保留此二进制资源，不提供文本预览。','This binary resource is retained. A text preview is unavailable.')}}</p><pre v-else-if="resourcePage" tabindex="0">{{resourceText}}</pre><button v-if="resourcePage?.hasMore" class="button ghost compact" :disabled="resourceLoading" @click="library.readResource(selectedResource,true)">{{tr('继续读取','Read more')}}</button><small class="skills-resource-digest" :title="selectedResource.sha256">SHA-256 · {{selectedResource.sha256.slice(0,16)}}</small></template>
      </aside>
    </div>
    <details class="skills-text-editor" v-if="!loading&&!error"><summary><FileText :size="17"/><strong>{{tr('文本指令与 JSON','Text instructions & JSON')}}</strong><span>{{tr('新建、编辑或导入现有指令','Create, edit, or import existing instructions')}}</span></summary><SkillInstructionsCatalog :catalog="textCatalog" standalone :exclude-skill-ids="packageSkillIds"/></details>
  </section>
</template>
<style scoped>
.skills-workspace{width:100%;min-width:0;color:var(--ui-ink);padding-bottom:20px}
.skills-workspace :is(button,input){font-family:var(--font-sans)}
.skills-workspace .button{min-height:40px;padding:8px 12px;font-size:13px;display:inline-flex;align-items:center;justify-content:center;gap:7px;white-space:nowrap}
.skills-workspace :is(button,summary,input):focus-visible{outline:2px solid var(--ui-accent);outline-offset:3px}
.skills-library-header{display:flex;align-items:center;justify-content:space-between;gap:20px;margin-bottom:18px}
.skills-library-intro{min-width:0}.skills-library-header h2{font-size:17px;font-weight:600;margin:0 0 5px;line-height:1.4}.skills-library-header p{font-size:13px;line-height:1.6;color:var(--ui-muted);margin:0}
.skills-header-actions{display:flex;align-items:center;gap:8px;flex-shrink:0}.skills-header-actions>.button:first-child{width:40px;padding:0}
.skills-eyebrow{display:flex;align-items:center;gap:7px;font-size:11px;color:var(--ui-muted)}
.skills-error{color:var(--red);font-size:13px;line-height:1.7;overflow-wrap:anywhere}.skills-muted{font-size:13px;color:var(--ui-muted);line-height:1.7}
.skills-library-toolbar{display:flex;align-items:center;justify-content:space-between;gap:16px;margin:0 0 16px;min-width:0}
.skills-library-count{font-size:12px;color:var(--ui-muted);line-height:1.5;text-align:right}
.skills-search{display:flex;align-items:center;gap:9px;flex:1;max-width:520px;min-width:0;height:40px;box-sizing:border-box;padding:0 12px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface);color:var(--ui-muted)}
.skills-search:focus-within{border-color:var(--ui-accent);box-shadow:0 0 0 3px color-mix(in srgb,var(--ui-accent) 12%,transparent)}
.skills-search>svg{flex-shrink:0}.skills-search input{border:0;outline:0;background:transparent;color:var(--ui-ink);width:100%;height:100%;min-width:0;padding:0;font-size:13px;box-shadow:none!important}
.skills-library-layout{display:grid;grid-template-columns:minmax(0,1fr);gap:16px;align-items:start}.skills-library-layout.with-resource{grid-template-columns:minmax(0,1fr) minmax(340px,1fr)}
.skills-package-list{min-width:0;display:grid;gap:12px}.skills-package{min-width:0;border:1px solid var(--ui-line);border-radius:var(--radius-lg);background:var(--ui-surface);overflow:hidden}
.skills-package-heading{display:flex;align-items:flex-start;gap:12px;padding:16px}.skills-package-icon{width:36px;height:36px;display:grid;place-items:center;flex-shrink:0;background:var(--ui-surface-raised);color:var(--ui-muted);border-radius:var(--radius-md)}
.skills-package-main{flex:1;min-width:0}.skills-package-title{display:flex;align-items:center;flex-wrap:wrap;gap:8px;min-width:0}.skills-package h3{font-size:15px;font-weight:600;line-height:1.5;margin:0;overflow-wrap:anywhere}
.skills-source-badge{padding:1px 7px;background:var(--ui-surface-raised);border-radius:var(--radius-pill);color:var(--ui-muted);font-size:11px;line-height:1.6;white-space:nowrap}
.skills-description-clamp{display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere;white-space:normal}
.skills-package-description{font-size:13px;line-height:1.65;color:var(--ui-muted);margin:6px 0 9px;max-width:78ch}
.skills-package-meta{display:flex;flex-wrap:wrap;align-items:center;gap:5px 14px;font-size:12px;line-height:1.5;color:var(--ui-muted)}.skills-match-count{color:var(--ui-accent)}
.skills-package-expand{flex-shrink:0;margin-left:auto;align-self:center}
.skills-package-info{padding:0 16px 12px 64px;font-size:12px;color:var(--ui-muted);min-width:0}.skills-package-info>summary{display:flex;align-items:center;gap:6px;list-style:none;cursor:pointer;width:fit-content;max-width:100%;line-height:1.6}.skills-package-info>summary::-webkit-details-marker{display:none}.skills-package-info>summary>svg{flex-shrink:0}.skills-package-info[open]>summary>svg:last-child{transform:rotate(180deg)}
.skills-package-info dl{display:grid;gap:9px;margin:14px 0;padding:12px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised)}.skills-package-info dl>div{display:grid;grid-template-columns:48px minmax(0,1fr);gap:12px}.skills-package-info dt{color:var(--ui-muted)}.skills-package-info dd{margin:0;color:var(--ui-copy);overflow-wrap:anywhere;line-height:1.6;white-space:pre-wrap}
.skills-capability-note{line-height:1.65;padding:0 0 4px}.skills-capability-note strong{font-weight:600;color:var(--ui-copy)}.skills-capability-note p{margin:5px 0}.skills-capability-note ul{margin:5px 0 0;padding-left:17px;overflow-wrap:anywhere}
.skills-members{border-top:1px solid var(--ui-line);min-width:0}.skills-members-heading{display:flex;justify-content:space-between;align-items:center;gap:10px;padding:10px 16px;font-size:11px;color:var(--ui-muted);background:var(--ui-surface-raised)}.skills-members-heading>span:first-child{font-weight:600;color:var(--ui-copy)}
.skills-member-list{max-height:440px;overflow:auto;overscroll-behavior:contain;min-width:0}
.skills-member{display:flex;align-items:center;gap:12px;padding:0 16px;border-top:1px solid var(--ui-line);min-width:0}.skills-member:first-child{border-top:0}.skills-member.selected{background:color-mix(in srgb,var(--ui-accent) 6%,var(--ui-surface))}.skills-member.disabled .skills-member-open{color:var(--ui-muted)}
.skills-member-open{display:flex;align-items:flex-start;gap:10px;flex:1;min-width:0;border:0;background:transparent;text-align:left;color:var(--ui-ink);padding:13px 0;cursor:pointer}.skills-member-open>svg{flex-shrink:0;margin-top:3px;color:var(--ui-muted)}.skills-member-open>svg:last-child{align-self:center;margin-top:0;margin-left:auto}.skills-member-open>span{min-width:0;flex:1}.skills-member-open strong{display:block;font-size:13px;line-height:1.5;font-weight:600;overflow-wrap:anywhere}.skills-member-open .skills-description-clamp{font-size:12px;line-height:1.6;color:var(--ui-muted);margin-top:4px}.skills-member-open:hover strong{color:var(--ui-accent)}
.skill-enabled-toggle{display:flex;align-items:center;gap:6px;flex-shrink:0;font-size:11px;color:var(--ui-muted);cursor:pointer}.skill-enabled-toggle input{margin:0;accent-color:var(--ui-accent)}
.skills-empty{text-align:center;padding:32px 16px;color:var(--ui-muted);border:1px dashed var(--ui-line);border-radius:var(--radius-lg)}.skills-empty h3{font-size:16px;font-weight:500;color:var(--ui-ink)}.skills-empty p{font-size:13px;line-height:1.7;max-width:440px;margin:0 auto}
.skills-bundled-install{display:flex;align-items:center;justify-content:space-between;gap:16px;padding:16px;border:1px dashed var(--ui-line);border-radius:var(--radius-lg)}.skills-bundled-install strong{font-size:13px}.skills-bundled-install p{font-size:12px;line-height:1.6;color:var(--ui-muted);margin:5px 0 0}
.skills-import-preview{padding:16px;border:1px solid var(--ui-accent);border-radius:var(--radius-lg);background:var(--ui-surface);margin-bottom:16px;min-width:0}.skills-import-preview>header{display:flex;align-items:center;justify-content:space-between;gap:16px}.skills-import-preview h3{font-size:16px;margin:5px 0 0}.skills-import-preview>p{font-size:12px;line-height:1.7;color:var(--ui-muted)}
.skills-preview-entries{list-style:none;padding:0;margin:12px 0;max-height:240px;overflow:auto}.skills-preview-entries li{padding:10px 0;border-top:1px solid var(--ui-line)}.skills-preview-entries strong{font-size:13px}.skills-preview-entries :is(span,p,code){font-size:12px;color:var(--ui-muted);line-height:1.6;overflow-wrap:anywhere}.skills-preview-entries summary{cursor:pointer}.skills-preview-entries summary>span{margin-top:4px}.skills-preview-entries p{white-space:pre-wrap}
.skills-import-preview footer{display:flex;align-items:center;justify-content:flex-end;flex-wrap:wrap;gap:8px;margin-top:16px}.skills-import-preview footer>span{margin-right:auto;font-size:11px;color:var(--ui-muted)}.skills-import-preview .skills-local-notice{font-size:12px}
.skills-capability-notice{display:flex;align-items:flex-start;gap:9px;padding:12px;border:1px solid color-mix(in srgb,var(--amber) 35%,var(--ui-line));border-radius:var(--radius-md);margin:12px 0;background:color-mix(in srgb,var(--amber) 5%,transparent)}.skills-capability-notice>svg{flex-shrink:0;color:var(--amber);margin-top:2px}.skills-capability-notice strong{font-size:12px}.skills-capability-notice li{font-size:12px;line-height:1.7;overflow-wrap:anywhere}.skills-capability-notice ul{padding-left:16px;margin:6px 0 0}
.skills-resource-panel{position:sticky;top:16px;min-width:0;padding:16px;border:1px solid var(--ui-line);border-radius:var(--radius-lg);background:var(--ui-surface);max-height:calc(100vh - 200px);overflow:auto;overscroll-behavior:contain}.skills-resource-panel>header{display:flex;align-items:flex-start;justify-content:space-between;gap:10px}.skills-resource-panel h3{font-size:15px;line-height:1.6;overflow-wrap:anywhere;margin:6px 0}.skills-resource-origin{font-size:12px;color:var(--ui-muted);line-height:1.6;overflow-wrap:anywhere;margin:0 0 12px}
.skills-full-description{border-top:1px solid var(--ui-line);border-bottom:1px solid var(--ui-line);padding:10px 0;font-size:12px;line-height:1.7;color:var(--ui-muted)}.skills-full-description>summary{cursor:pointer;color:var(--ui-copy);font-weight:600}.skills-full-description p{max-height:200px;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere;margin:8px 0 0;padding-right:8px}
.skills-resource-files{max-height:190px;overflow:auto;margin:12px 0;display:flex;flex-direction:column;gap:3px}.skills-resource-files button{display:flex;align-items:center;gap:7px;min-width:0;padding:8px;border:0;background:transparent;color:var(--ui-muted);text-align:left;border-radius:var(--radius-sm);cursor:pointer}.skills-resource-files button.active{background:color-mix(in srgb,var(--ui-accent) 9%,transparent);color:var(--ui-ink)}.skills-resource-files button>svg{flex-shrink:0}.skills-resource-files span{font-size:12px;line-height:1.5;overflow-wrap:anywhere;flex:1}.skills-resource-files small{font-size:11px;white-space:nowrap}
.skills-file-heading{display:flex;align-items:flex-start;justify-content:space-between;gap:8px;padding:10px 0;border-top:1px solid var(--ui-line);font-size:12px}.skills-file-heading strong{overflow-wrap:anywhere;line-height:1.6}.skills-file-heading span{white-space:nowrap;color:var(--ui-muted)}.skills-resource-panel pre{white-space:pre-wrap;overflow-wrap:anywhere;font:12px/1.7 var(--font-mono);max-height:360px;overflow:auto;margin:0 0 12px;padding:12px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised)}.skills-script-notice{font-size:12px;line-height:1.6;color:var(--ui-muted)}.skills-resource-digest{display:block;font-size:11px;color:var(--ui-muted);margin-top:10px}
.skills-text-editor{margin-top:20px;border-top:1px solid var(--ui-line);padding-top:14px}.skills-text-editor>summary{display:flex;align-items:center;gap:9px;cursor:pointer;list-style:none;margin-bottom:14px;min-width:0}.skills-text-editor>summary::-webkit-details-marker{display:none}.skills-text-editor summary strong{font-size:13px;font-weight:600}.skills-text-editor summary span{font-size:12px;color:var(--ui-muted);margin-left:auto}
@media(max-width:1100px){.skills-library-layout.with-resource{grid-template-columns:minmax(0,1fr)}.skills-resource-panel{position:relative;top:0;max-height:720px}.skills-members-heading>span:last-child{display:none}}
@media(max-width:700px){.skills-library-header{align-items:flex-start;flex-direction:column;gap:12px}.skills-header-actions{align-self:flex-end}.skills-library-toolbar{align-items:stretch;flex-direction:column;gap:8px}.skills-library-count{text-align:left}.skills-search{flex:auto;max-width:none;width:100%}.skills-package-heading{gap:10px;flex-wrap:wrap}.skills-package-main{flex-basis:calc(100% - 48px)}.skills-package-expand{margin-left:46px;align-self:flex-start}.skills-package-info{padding-left:16px}.skills-member{padding:0 12px;gap:8px}.skill-enabled-toggle>span{display:none}.skills-bundled-install{align-items:flex-start;flex-direction:column}.skills-text-editor summary span{display:none}}
</style>
