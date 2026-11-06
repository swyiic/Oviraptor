import { computed, ref, watch } from "vue";
import { useI18n } from "../../i18n";
import type { AgentTaskEvidence } from "./types";

/**
 * One row in the live tool-activity timeline shown under a running/completed
 * assistant reply. Built client-side from whatever the host already streams
 * (`runtime.step` titles, `runtime.heartbeat` elapsed, `runtime.inner`
 * reasoning snapshots — now carrying a monotonic `seq` + host-clock `at` — and
 * `task.evidence` receipts, which carry real startedAt/finishedAt/status and,
 * once finished, a `previewText`) — there is no dedicated per-step backend
 * event yet. Still missing (tracked in docs/AGENT_TOP_LEVEL_REFACTOR.md §7):
 * a live "latest output" preview for a tool step that is still running.
 */
export type ToolTimelineStep = {
  id: string;
  kind: "tool" | "thinking";
  /** Tool/action name, or the reasoning phase label for a thinking row. */
  title: string;
  status: "running" | "done" | "failed" | "thinking";
  /** One-line human summary of what this step is/was doing. */
  actionSummary: string;
  /** Longer text shown when the row is expanded (receipt summary or reasoning detail). */
  detail?: string;
  startedAtMs?: number;
  elapsedMs: number;
};

type InnerSnapshot = { goal: string; phase: string; phaseLabel: string; facts: string[]; hypotheses: string[]; compressed: boolean; hold: string; seq?: number; at?: number } | null;

let uid = 0;
const nextId = (prefix: string) => `${prefix}-${++uid}`;

export function useToolTimeline(getters: {
  steps: () => string[];
  heartbeat: () => { elapsedMs: number; stepTitle?: string } | undefined;
  inner: () => InnerSnapshot;
  evidence: () => AgentTaskEvidence[];
  running: () => boolean;
}) {
  const { tr } = useI18n();
  const now = ref(Date.now());
  let ticker: number | undefined;
  watch(getters.running, (value) => {
    if (ticker !== undefined) { window.clearInterval(ticker); ticker = undefined; }
    if (value) { now.value = Date.now(); ticker = window.setInterval(() => { now.value = Date.now(); }, 1000); }
  }, { immediate: true });

  // `inner` is a single overwritten snapshot (not a history), so each
  // meaningfully-different snapshot is appended here to recover a "thinking"
  // trail. `at` is the host's own Date.now() at the moment it thought this —
  // the same clock task.evidence's startedAt/finishedAt come from — so it
  // merges precisely against tool steps instead of by client-side guesswork.
  // Cleared whenever `resetActivity()` drops `steps` back to empty.
  const thinkingLog = ref<{ id: string; title: string; detail: string; at: number; seq: number }[]>([]);
  let lastThinkingKey = "";
  watch(getters.inner, (value) => {
    if (!value) return;
    if (!value.phaseLabel && !value.goal && !value.facts.length && !value.hypotheses.length && !value.hold) return;
    const key = `${value.phaseLabel}|${value.goal}|${value.facts.join(",")}|${value.hypotheses.join(",")}|${value.hold}`;
    if (key === lastThinkingKey) return;
    lastThinkingKey = key;
    thinkingLog.value = [...thinkingLog.value, {
      id: nextId("think"),
      title: value.phaseLabel || value.goal,
      detail: [value.goal, value.hold, ...value.facts, ...value.hypotheses].filter(Boolean).join(" · "),
      at: value.at ?? Date.now(),
      seq: value.seq ?? 0,
    }].slice(-8);
  });
  watch(() => getters.steps().length, (length, previous) => {
    if (length === 0 && (previous ?? 0) > 0) { thinkingLog.value = []; lastThinkingKey = ""; }
  });

  /** Evidence (task.evidence) has real timestamps and status; prefer it whenever the
   *  host has reported at least one receipt for this run. Bare step titles (no timing,
   *  no per-step status) are the fallback for the gap before the first receipt lands. */
  const toolSteps = computed<ToolTimelineStep[]>(() => {
    const evidence = getters.evidence();
    const running = getters.running();
    if (evidence.length) {
      return evidence.map((item, index) => {
        const startedAtMs = item.startedAt ? Date.parse(item.startedAt) : undefined;
        const finishedAtMs = item.finishedAt ? Date.parse(item.finishedAt) : undefined;
        const isRunning = !finishedAtMs && index === evidence.length - 1 && running;
        const elapsedMs = startedAtMs === undefined ? 0 : (finishedAtMs ?? (isRunning ? now.value : startedAtMs)) - startedAtMs;
        const memoryTitle = item.toolName === "remember" ? tr("🧠 记住了一条跨会话记忆", "🧠 Saved a cross-session memory")
          : item.toolName === "forget" ? tr("🗑 删掉了一条记忆", "🗑 Removed a memory") : undefined;
        // previewText (first 200 chars of summary, whitespace-collapsed) is empty while
        // running and set once the step finishes — use it for the one-line row label,
        // and keep the full summary for the expanded detail (hidden when identical).
        const oneLine = item.toolName === "remember" ? tr("在配置中心·模型里查看或删除", "View or delete in Config Center · Models") : (item.previewText || item.summary || item.toolName);
        return {
          id: item.id,
          kind: "tool" as const,
          title: memoryTitle || item.toolName,
          status: isRunning ? "running" : item.status === "failed" ? "failed" : "done",
          actionSummary: oneLine,
          detail: item.summary || undefined,
          startedAtMs,
          elapsedMs: Math.max(0, elapsedMs),
        };
      });
    }
    const steps = getters.steps();
    const heartbeat = getters.heartbeat();
    return steps.map((title, index) => {
      const isRunning = index === steps.length - 1 && running;
      return {
        id: nextId("step"),
        kind: "tool" as const,
        title,
        status: isRunning ? "running" : "done",
        actionSummary: title,
        elapsedMs: isRunning ? (heartbeat?.elapsedMs ?? 0) : 0,
      };
    });
  });

  const timeline = computed<ToolTimelineStep[]>(() => {
    const tools: Array<ToolTimelineStep & { seq: number }> = toolSteps.value.map(step => ({ ...step, seq: 0 }));
    const thinks: Array<ToolTimelineStep & { seq: number }> = thinkingLog.value.map(item => ({
      id: item.id, kind: "thinking" as const, title: item.title, status: "thinking" as const,
      actionSummary: item.detail, detail: item.detail, elapsedMs: 0, startedAtMs: item.at, seq: item.seq,
    }));
    if (!thinks.length) return tools;
    if (!tools.length) return thinks;
    // When every tool step carries a real timestamp (the evidence path), both
    // sides share the host's own clock (task.evidence's startedAt and inner's
    // `at` are both server-side Date.now()), so a chronological merge places
    // thinking rows precisely between tool steps — `seq` breaks exact ties.
    // Bare titles (no timing) have no sound ordering signal, so thinking rows
    // are appended after the tool steps there instead of guessed.
    if (tools.every(step => step.startedAtMs !== undefined)) {
      return [...tools, ...thinks].sort((a, b) => (a.startedAtMs ?? 0) - (b.startedAtMs ?? 0) || a.seq - b.seq);
    }
    return [...tools, ...thinks];
  });

  const totalElapsedMs = computed(() => {
    if (!getters.running()) return 0;
    const started = toolSteps.value.find(step => step.startedAtMs !== undefined)?.startedAtMs;
    return started !== undefined ? now.value - started : getters.heartbeat()?.elapsedMs ?? 0;
  });

  return { timeline, totalElapsedMs };
}
