<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";

const props = defineProps<{ scanId: string }>();
const { tr } = useI18n();

interface PendingQuestion {
  questionId: string;
  attemptNumber: number;
  question: string;
  options: string[];
  context: string;
  createdAt: string;
}

const pending = ref<PendingQuestion[]>([]);
const answers = ref<Record<string, string>>({});
const selected = ref<Record<string, string>>({});
const busy = ref("");
const error = ref("");
let timer: ReturnType<typeof setInterval> | undefined;
let disposed = false;

async function refresh() {
  if (!props.scanId) {
    pending.value = [];
    return;
  }
  try {
    const rows = (await sentinelApi.listPendingOperatorQuestions(props.scanId)) as PendingQuestion[];
    if (!disposed) {
      pending.value = rows;
      error.value = "";
    }
  } catch (e) {
    if (!disposed) error.value = String(e);
  }
}

async function submit(item: PendingQuestion) {
  const text = (answers.value[item.questionId] ?? selected.value[item.questionId] ?? "").trim();
  if (!text || busy.value) return;
  busy.value = item.questionId;
  error.value = "";
  try {
    await sentinelApi.answerOperatorQuestion(item.questionId, text, selected.value[item.questionId]);
    delete answers.value[item.questionId];
    delete selected.value[item.questionId];
    await refresh();
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function dismiss(questionId: string) {
  if (busy.value) return;
  busy.value = questionId;
  try {
    await sentinelApi.dismissOperatorQuestion(questionId);
    await refresh();
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

onMounted(() => {
  void refresh();
  timer = setInterval(refresh, 4000);
});
onUnmounted(() => {
  disposed = true;
  if (timer) clearInterval(timer);
});
watch(() => props.scanId, () => refresh());
</script>

<template>
  <section v-if="pending.length" class="operator-question-inbox">
    <header>
      <strong>{{ tr("Agent 向你提问", "Agent questions for you") }}</strong>
      <small>{{ tr("结构化回答会回传给 Agent；关闭后 Agent 需改走其它路径。", "Structured answers return to the agent; dismiss sends it elsewhere.") }}</small>
    </header>
    <ul>
      <li v-for="item in pending" :key="item.questionId">
        <p class="oq-text">{{ item.question }}</p>
        <p v-if="item.context" class="oq-context">{{ item.context }}</p>
        <div v-if="item.options.length" class="oq-options">
          <button
            v-for="option in item.options"
            :key="option"
            type="button"
            :class="{ active: selected[item.questionId] === option }"
            @click="selected[item.questionId] = option; answers[item.questionId] = option"
          >
            {{ option }}
          </button>
        </div>
        <textarea v-model="answers[item.questionId]" rows="2" :placeholder="tr('自由回答…', 'Free-form answer…')" />
        <div class="oq-actions">
          <button type="button" class="button primary" :disabled="busy === item.questionId" @click="submit(item)">
            {{ tr("提交回答", "Submit answer") }}
          </button>
          <button type="button" class="button ghost" :disabled="busy === item.questionId" @click="dismiss(item.questionId)">
            {{ tr("关闭", "Dismiss") }}
          </button>
        </div>
      </li>
    </ul>
    <small v-if="error" class="oq-error">{{ error }}</small>
  </section>
</template>

<style scoped>
.operator-question-inbox {
  margin: 12px 0;
  padding: 14px 16px;
  border: 1px solid color-mix(in srgb, var(--ui-accent, #3b6) 35%, var(--border, #d0d0d0));
  border-radius: 10px;
  background: color-mix(in srgb, var(--ui-accent, #3b6) 6%, var(--surface, #fff));
  display: flex;
  flex-direction: column;
  gap: 10px;
}
header { display: flex; flex-direction: column; gap: 3px; }
header small { color: var(--muted, #6a6a6a); font-size: 0.8rem; line-height: 1.5; }
ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 10px; }
li {
  padding: 10px 12px;
  border: 1px solid var(--border, #e0e0e0);
  border-radius: 8px;
  background: var(--surface, #fff);
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.oq-text { margin: 0; font-weight: 600; line-height: 1.5; }
.oq-context { margin: 0; color: var(--muted, #666); font-size: 0.85rem; }
.oq-options { display: flex; flex-wrap: wrap; gap: 6px; }
.oq-options button {
  border: 1px solid var(--border, #ccc);
  border-radius: 999px;
  padding: 4px 10px;
  background: transparent;
  cursor: pointer;
  font: inherit;
}
.oq-options button.active { border-color: var(--ui-accent, #3b6); background: color-mix(in srgb, var(--ui-accent, #3b6) 12%, transparent); }
textarea { width: 100%; box-sizing: border-box; font: inherit; border-radius: 6px; border: 1px solid var(--border, #ddd); padding: 8px; }
.oq-actions { display: flex; gap: 8px; }
.oq-error { color: #b4332f; font-size: 0.78rem; }
</style>
