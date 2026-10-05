<script setup>
import { ref } from 'vue';
import AgentDialog from '../../src/features/sentinel/components/AgentDialog.vue';
import { metrics, burst, newAttempt, malformed, release } from './transport.mjs';
const mounted = ref(true);
</script>

<template>
  <main class="fixture">
    <section class="fixture-controls" aria-label="本地验收控制台">
      <h1>真实聊天组件 · 本地 DOM 验收</h1>
      <p>全部消息、任务、回执均为模拟数据。无数据库、Tauri IPC、模型、扫描或外部目标请求；解析/审批故意拒绝。刷新清空夹具状态。</p>
      <div class="fixture-actions">
        <button @click="burst('A')">A 突发 200 条</button>
        <button @click="burst('B')">B 突发 200 条</button>
        <button @click="newAttempt('A')">A 新 attempt</button>
        <button @click="malformed">发送畸形通知</button>
        <button @click="metrics.hold = !metrics.hold">{{ metrics.hold ? '关闭延迟' : '开启延迟' }}</button>
        <button :disabled="!metrics.pending.length" @click="release()">释放最早请求</button>
        <button :disabled="!metrics.pending.length" @click="release(true)">释放最新请求</button>
        <button @click="metrics.failNext = true">下次读取失败</button>
        <button @click="mounted = !mounted">{{ mounted ? '卸载组件' : '重新挂载' }}</button>
      </div>
      <output aria-label="读取指标">读取 {{ metrics.reads }} · 在途 {{ metrics.active }} · 峰值 {{ metrics.peak }} · 待释放 {{ metrics.pending.join(', ') || '无' }}</output>
      <p role="status">浏览器错误：{{ metrics.errors.join('；') || '无' }}</p>
      <details><summary>读取记录</summary><pre>{{ metrics.calls.join('\n') }}</pre></details>
    </section>
    <AgentDialog v-if="mounted" :project-id="1" initial-scan-id="A" />
  </main>
</template>

<style scoped>
.fixture { max-width: 1440px; margin: 0 auto; padding: 20px; }
.fixture-controls { margin-bottom: 20px; padding: 16px; border: 2px solid #9b6400; border-radius: 12px; background: #fff9e9; }
h1 { font-size: 20px; margin: 0; }
.fixture-controls p { font-size: 13px; }
.fixture-actions { display: flex; flex-wrap: wrap; gap: 8px; margin: 12px 0; }
button { padding: 6px 10px; }
output { display: block; font-variant-numeric: tabular-nums; }
pre { max-height: 180px; overflow: auto; }
</style>
