import { onUnmounted, ref, watch } from "vue";

interface TaskSearchOptions {
  query: () => string;
  scope: () => number | undefined;
  lookup: (query: string) => Promise<string[]>;
  showResults: () => void;
  selectMatch: () => Promise<void>;
  onError: (message: string) => void;
}

// Read-only task lookup. Each query/scope change invalidates older results,
// including A -> B -> A transitions and clearing an outstanding search.
export function useTaskSearch(options: TaskSearchOptions) {
  const matchedScanIds = ref<string[]>([]);
  let generation = 0;
  let disposed = false;

  async function applySearch(value: string) {
    if (disposed) return;
    const request = ++generation;
    const scope = options.scope();
    const current = () => !disposed && request === generation
      && value === options.query() && scope === options.scope();
    matchedScanIds.value = [];
    if (!value.trim()) return;
    options.showResults();
    try {
      const matches = await options.lookup(value);
      if (!current()) return;
      if (!Array.isArray(matches) || matches.some(id => typeof id !== "string" || !id.trim())) {
        throw new Error("invalid_task_search_result");
      }
      matchedScanIds.value = [...matches];
      await options.selectMatch();
    } catch {
      if (current()) {
        matchedScanIds.value = [];
        options.onError("任务查询失败，请重试");
      }
    }
  }

  watch([options.query, options.scope], ([query]) => {
    void applySearch(query);
  }, { flush: "sync" });

  onUnmounted(() => { disposed = true; ++generation; });
  return { matchedScanIds, applySearch };
}
