<script setup lang="ts">
import { computed } from "vue";
import { BookOpen, Braces, Download, Plus, RefreshCw, Save, Sparkles, Trash2, Upload, X } from "@lucide/vue";
import InlineConfirm from "../../shared/components/InlineConfirm.vue";
import { useI18n } from "../../i18n";
import type { useSkillsCatalog } from "./useSkillsCatalog";

const props = defineProps<{ catalog: ReturnType<typeof useSkillsCatalog>; standalone?: boolean; excludeSkillIds?: number[] }>();
const { tr } = useI18n();
const { skills, skillBusy, refiningSkillId, deleteSkill, editingSkill, showSkillEditor,
  skillForm, skillPreview, refineSkill, exportSkills, importSkills,
  importInternalSecSkills, editSkill, cloneBuiltinSkill, insertSkillTemplate,
  saveSkill, removeSkill } = props.catalog;
const visibleSkills = computed(() => skills.value.filter(skill => !props.excludeSkillIds?.includes(skill.id)));
</script>

<template>
  <div class="text-skills">
    <div class="text-skills-bar">
      <div>
        <h3>{{ standalone?tr("文本指令", "Text instructions"):tr("技能指令", "Skill instructions") }}</h3>
        <p>{{ standalone?tr("技能包在上面管理。这里只新建、导入你自己的文本指令。", "Package skills stay above. This section is only for your own text instructions."):tr("每个任务自行选择 Skill。", "Each task chooses its own skills.") }}</p>
      </div>
      <div class="text-skills-actions">
        <button class="button ghost compact" type="button" @click="importSkills"><Upload :size="14" />{{ standalone?tr("导入 JSON", "Import JSON"):tr("导入", "Import") }}</button>
        <button v-if="!standalone" class="button ghost compact" type="button" @click="importInternalSecSkills"><BookOpen :size="14" />{{ tr("导入内部 sec_skills", "Import internal sec_skills") }}</button>
        <button class="button ghost compact" type="button" @click="exportSkills"><Download :size="14" />{{ tr("导出", "Export") }}</button>
        <button class="button primary compact" type="button" @click="editSkill()"><Plus :size="14" />{{ tr("新增技能", "New skill") }}</button>
      </div>
    </div>
    <p v-if="!visibleSkills.length && !showSkillEditor" class="text-skills-empty">{{ tr("还没有自己的文本指令。", "No text instructions yet.") }}</p>
    <article v-for="skill in visibleSkills" :key="skill.id" class="text-skill-row">
      <div>
        <strong>{{ skill.name }}</strong>
        <small>{{ skill.builtin ? tr("内置", "Built-in") : tr("自定义", "Custom") }} · {{ skill.enabled ? tr("已启用", "Enabled") : tr("已停用", "Disabled") }}</small>
        <p>{{ skill.description }}</p>
        <details><summary>{{ tr("查看指令", "View instructions") }}</summary><pre>{{ skill.instructions }}</pre></details>
      </div>
      <div class="text-skills-actions">
        <button v-if="!standalone" class="button ghost compact" type="button" :disabled="refiningSkillId === skill.id" @click="refineSkill(skill)">
          <Sparkles v-if="refiningSkillId !== skill.id" :size="13" />
          <RefreshCw v-else :size="13" class="spinning" />
          {{ refiningSkillId === skill.id ? tr("精炼中", "Refining") : tr("用最新知识精炼", "Refine with latest") }}
        </button>
        <button v-if="skill.builtin" class="button ghost compact" type="button" @click="cloneBuiltinSkill(skill)">{{ tr("复制后编辑", "Clone & edit") }}</button>
        <button v-else class="button ghost compact" type="button" @click="editSkill(skill)">{{ tr("编辑", "Edit") }}</button>
        <button v-if="!skill.builtin" class="button ghost compact" type="button" @click="deleteSkill = skill"><Trash2 :size="13" />{{ tr("删除", "Delete") }}</button>
      </div>
      <InlineConfirm v-if="deleteSkill?.id === skill.id"
        :title="tr(`删除技能「${skill.name}」？`, `Delete skill “${skill.name}”?`)"
        :detail="tr('历史任务不受影响，新任务将不再加载该技能。', 'Existing tasks are unchanged; new tasks will no longer load it.')"
        :busy="skillBusy" @cancel="deleteSkill = undefined" @confirm="removeSkill" />
    </article>
    <section v-if="showSkillEditor" class="text-skill-editor">
      <header>
        <h3>{{ editingSkill ? tr("编辑自定义技能", "Edit custom skill") : tr("新增自定义技能", "New custom skill") }}</h3>
        <div class="text-skills-actions">
          <button class="button ghost compact" type="button" @click="insertSkillTemplate"><Braces :size="14" />{{ tr("插入规范模板", "Insert template") }}</button>
          <button class="icon-button" type="button" :aria-label="tr('关闭', 'Close')" @click="showSkillEditor = false"><X :size="15" /></button>
        </div>
      </header>
      <label class="text-skill-field"><span>{{ tr("名称", "Name") }}</span><input v-model="skillForm.name" :placeholder="tr('例如：Java Spring 鉴权审计', 'e.g. Java Spring auth review')" /></label>
      <label class="text-skill-field"><span>{{ tr("说明", "Description") }}</span><input v-model="skillForm.description" :placeholder="tr('一句话说明适用范围', 'One-line use case')" /></label>
      <div class="text-skill-guide">
        <strong>{{ tr("推荐结构", "Recommended structure") }}</strong>
        <span>Objective → Scope → Analysis workflow → Output requirements</span>
        <p>{{ tr("写清检查目标、范围、验证步骤和输出。不要要求跳过授权或把普通信息当漏洞。", "State the objective, scope, checks, and output. Do not ask to skip authorization or treat ordinary notes as vulnerabilities.") }}</p>
      </div>
      <label class="text-skill-field"><span>{{ standalone?tr("指令内容", "Instructions"):tr("任务选用的指令", "Instructions selected by a task") }}</span>
        <textarea v-model="skillForm.instructions" rows="12" :placeholder="tr('点击“插入规范模板”开始填写', 'Use Insert template to start')"></textarea>
      </label>
      <div class="text-skill-preview">
        <header><strong>{{ tr("格式化预览", "Formatted preview") }}</strong><small>{{ skillPreview.length }}</small></header>
        <article v-for="section in skillPreview" :key="section.heading"><h4>{{ section.heading }}</h4><pre>{{ section.body }}</pre></article>
        <p v-if="!skillPreview.length">{{ tr("输入指令后这里会显示章节。", "Sections appear here as you type.") }}</p>
      </div>
      <label class="text-skill-check"><input v-model="skillForm.enabled" type="checkbox" />{{ tr("启用", "Enabled") }}</label>
      <footer>
        <button class="button ghost compact" type="button" @click="showSkillEditor = false">{{ tr("取消", "Cancel") }}</button>
        <button class="button primary compact" type="button" :disabled="skillBusy" @click="saveSkill"><Save :size="14" />{{ tr("保存技能", "Save skill") }}</button>
      </footer>
    </section>
  </div>
</template>
<style scoped>
.text-skills{display:flex;flex-direction:column;gap:14px;min-width:0}
.text-skills-bar{display:flex;align-items:flex-start;justify-content:space-between;gap:16px}
.text-skills-bar h3{margin:0 0 4px;font-size:16px;font-weight:600}
.text-skills-bar p,.text-skills-empty,.text-skill-row p,.text-skill-guide p,.text-skill-preview p{margin:0;color:var(--app-muted,#72746f);font-size:12px;line-height:1.6}
.text-skills-actions{display:flex;flex-wrap:wrap;gap:8px;flex-shrink:0}
.text-skill-row{display:flex;align-items:flex-start;justify-content:space-between;gap:16px;padding:14px 0;border-top:1px solid var(--line,#e5e6e1)}
.text-skill-row>div:first-child{min-width:0;flex:1}
.text-skill-row strong{display:block;font-size:14px}
.text-skill-row small{display:block;margin-top:3px;color:var(--app-muted,#72746f);font-size:11px}
.text-skill-row details{margin-top:8px}
.text-skill-row summary{cursor:pointer;color:var(--app-ink,#252624);font-size:12px}
.text-skill-row pre,.text-skill-preview pre{white-space:pre-wrap;overflow:auto;max-height:220px;margin:8px 0 0;padding:10px;border:1px solid var(--line,#e5e6e1);border-radius:8px;background:var(--app-bg,#fcfcfb);font:12px/1.6 ui-monospace,SFMono-Regular,monospace}
.text-skill-editor{display:flex;flex-direction:column;gap:12px;padding-top:8px;border-top:1px solid var(--line,#e5e6e1)}
.text-skill-editor>header{display:flex;align-items:center;justify-content:space-between;gap:12px}
.text-skill-editor h3{margin:0;font-size:16px}
.text-skill-field{display:flex;flex-direction:column;gap:6px}
.text-skill-field>span{font-size:12px;color:var(--app-muted,#72746f)}
.text-skill-field input,.text-skill-field textarea{width:100%;border:1px solid var(--line,#e5e6e1);border-radius:8px;background:var(--app-surface,#fff);color:var(--app-ink,#252624);box-shadow:none!important;font:inherit;font-size:13px}
.text-skill-field input{height:36px;padding:0 10px}
.text-skill-field textarea{min-height:180px;padding:10px;resize:vertical;line-height:1.6}
.text-skill-guide,.text-skill-preview{padding:12px 14px;border:1px solid var(--line,#e5e6e1);border-radius:10px;background:var(--app-bg,#fcfcfb)}
.text-skill-guide{display:flex;flex-direction:column;gap:4px}
.text-skill-guide strong,.text-skill-preview strong{font-size:12px}
.text-skill-guide span,.text-skill-preview small{color:var(--app-muted,#72746f);font-size:12px}
.text-skill-preview header{display:flex;justify-content:space-between;gap:8px}
.text-skill-preview h4{margin:10px 0 0;font-size:13px}
.text-skill-check{display:flex;align-items:center;gap:8px;font-size:13px}
.text-skill-editor>footer{display:flex;justify-content:flex-end;gap:8px}
@media(max-width:760px){.text-skills-bar,.text-skill-row{flex-direction:column}}
</style>
