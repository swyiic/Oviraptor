import { onMounted, onBeforeUnmount, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// Committed-data events are hints. Callers read authoritative, bounded snapshots
// and fence their responses by selection/generation; payload text is never used.
export function useCommittedRefresh(
  channel: string,
  scopeKey: () => string,
  accepts: (payload: unknown) => boolean,
  busy: () => boolean,
  refresh: () => Promise<void>,
  connectionState: (unavailable: boolean) => void,
) {
  let mounted = false, inFlight = false, subscribing = false;
  let pending = "";
  let timer: number | undefined, fallback: number | undefined;
  let release: UnlistenFn | undefined;

  function schedule() {
    if (!mounted || document.hidden || inFlight || busy() || !pending || timer !== undefined) return;
    timer = window.setTimeout(() => { timer = undefined; void reconcile(); }, 50);
  }
  function catchUp() { pending = scopeKey(); schedule(); }
  async function reconcile() {
    if (!mounted || document.hidden || inFlight || busy()) return;
    const key = scopeKey();
    if (!key || pending !== key) { pending = ""; return; }
    pending = ""; inFlight = true;
    try { await refresh(); }
    catch { /* The snapshot owner retains data and reports read errors. */ }
    finally { inFlight = false; schedule(); }
  }
  async function subscribe() {
    if (!mounted || release || subscribing) return;
    subscribing = true;
    try {
      const unlisten = await listen<unknown>(channel, ({ payload }) => {
        if (mounted && scopeKey() && accepts(payload)) catchUp();
      });
      if (!mounted) unlisten();
      else { release = unlisten; connectionState(false); catchUp(); }
    } catch { if (mounted) connectionState(true); }
    finally { subscribing = false; }
  }
  function recover() { if (!document.hidden) { void subscribe(); catchUp(); } }
  // Observe every selection change, including A -> B -> A in one Vue tick.
  watch(scopeKey, catchUp, { flush: "sync" });
  watch(busy, value => { if (!value) schedule(); });
  onMounted(() => {
    mounted = true;
    document.addEventListener("visibilitychange", recover);
    void subscribe();
    // Event delivery is primary; this only reconciles dropped/late notifications.
    fallback = window.setInterval(recover, 15000);
  });
  onBeforeUnmount(() => {
    mounted = false; pending = "";
    if (timer !== undefined) window.clearTimeout(timer);
    if (fallback !== undefined) window.clearInterval(fallback);
    document.removeEventListener("visibilitychange", recover);
    release?.();
  });
  return { catchUp };
}
