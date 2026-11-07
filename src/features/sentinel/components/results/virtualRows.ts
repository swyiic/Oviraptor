import { computed, onBeforeUnmount, ref, watch, type Ref } from "vue";

/**
 * Spacer-based dynamic-height virtualization for a list that owns its own bounded,
 * scrollable container (same technique as AgentHostChat.vue's transcript — real
 * spacer divs keep scrollHeight/scrollTop native and accurate, no absolute positioning).
 * Unlike the transcript, these lists have no scroll-anchor requirement, so there is
 * no need to track an external scroll position across data reloads beyond reset().
 */
export function useVirtualRows<T>(
  itemsRef: Ref<T[]>,
  keyOf: (item: T) => string | number,
  opts: { defaultHeight?: number; buffer?: number; gap?: number; initialRender?: number } = {},
) {
  const defaultHeight = opts.defaultHeight ?? 120;
  const buffer = opts.buffer ?? 600;
  const gap = opts.gap ?? 0;
  const initialRender = opts.initialRender ?? 40;

  let containerEl: HTMLElement | undefined;
  const measured = new Map<string, number>();
  const rowEls = new Map<string, HTMLElement>();
  let avgHeight = defaultHeight;
  let resizeObserver: ResizeObserver | undefined;

  const windowStart = ref(0), windowEnd = ref(0), spacerBefore = ref(0), spacerAfter = ref(0);
  const windowed = computed(() => itemsRef.value.slice(windowStart.value, windowEnd.value));

  function recomputeAverage() {
    if (!measured.size) return;
    let sum = 0; for (const h of measured.values()) sum += h;
    avgHeight = sum / measured.size;
  }
  function strideFor(key: string) { return (measured.get(key) ?? avgHeight) + gap; }
  function lowerBound(offsets: number[], value: number): number {
    let lo = 0, hi = offsets.length - 1;
    while (lo < hi) { const mid = (lo + hi) >> 1; if (offsets[mid] < value) lo = mid + 1; else hi = mid; }
    return lo;
  }
  function update() {
    const list = itemsRef.value, total = list.length;
    if (!total) { windowStart.value = 0; windowEnd.value = 0; spacerBefore.value = 0; spacerAfter.value = 0; return; }
    if (!containerEl) { windowStart.value = 0; windowEnd.value = Math.min(total, initialRender); spacerBefore.value = 0; spacerAfter.value = 0; return; }
    const offsets = new Array(total + 1); offsets[0] = 0;
    for (let i = 0; i < total; i++) offsets[i + 1] = offsets[i] + strideFor(String(keyOf(list[i])));
    const viewTop = Math.max(0, containerEl.scrollTop - buffer);
    const viewBottom = containerEl.scrollTop + containerEl.clientHeight + buffer;
    const start = Math.max(0, lowerBound(offsets, viewTop) - 1);
    const end = Math.min(total, Math.max(start, lowerBound(offsets, viewBottom)));
    windowStart.value = start; windowEnd.value = end;
    spacerBefore.value = offsets[start];
    spacerAfter.value = offsets[total] - offsets[end];
  }

  const rowCallbacks = new Map<string, (el: unknown) => void>();
  function bindRow(key: string | number) {
    const stringKey = String(key);
    let fn = rowCallbacks.get(stringKey);
    if (!fn) {
      fn = (el: unknown) => {
        if (el instanceof HTMLElement) { el.dataset.vkey = stringKey; rowEls.set(stringKey, el); resizeObserver?.observe(el); }
        else { const prev = rowEls.get(stringKey); if (prev) { resizeObserver?.unobserve(prev); rowEls.delete(stringKey); } }
      };
      rowCallbacks.set(stringKey, fn);
    }
    return fn;
  }
  function setContainer(el: unknown) {
    containerEl = el instanceof HTMLElement ? el : undefined;
    if (containerEl && typeof ResizeObserver !== "undefined" && !resizeObserver) {
      resizeObserver = new ResizeObserver(entries => {
        let changed = false;
        for (const entry of entries) {
          const key = (entry.target as HTMLElement).dataset.vkey;
          if (!key) continue;
          const height = entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height;
          const prev = measured.get(key);
          if (prev === undefined || Math.abs(prev - height) > 1) { measured.set(key, height); changed = true; }
        }
        if (changed) { recomputeAverage(); update(); }
      });
      for (const rowEl of rowEls.values()) resizeObserver.observe(rowEl);
    }
    update();
  }
  function onScroll() { update(); }
  function reset() {
    measured.clear(); rowCallbacks.clear(); avgHeight = defaultHeight;
    if (containerEl) containerEl.scrollTop = 0;
    update();
  }
  watch(itemsRef, () => update());
  onBeforeUnmount(() => resizeObserver?.disconnect());

  return { windowed, spacerBefore, spacerAfter, setContainer, bindRow, onScroll, reset };
}
