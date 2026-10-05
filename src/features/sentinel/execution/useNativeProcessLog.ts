import { onBeforeUnmount, ref, watch } from "vue";
import { sentinelApi } from "../api";
import { useCommittedRefresh } from "../../../composables/useCommittedRefresh";
import type { LogScope } from "./useLiveRunnerLog";
import { acceptNativePage, acceptsNativeHint, emptyNativeReplay } from "./nativeProcessLogContract";


export function useNativeProcessLog(selected: () => LogScope | undefined) {
  const replay = ref(emptyNativeReplay()), loading = ref(false), readFailed = ref(false), connectionUnavailable = ref(false);
  let generation = 0, disposed = false;
  const key = () => { const s = selected(); return s ? JSON.stringify([s.scanId, s.attempt]) : ""; };
  watch(key, () => {
    ++generation; replay.value = emptyNativeReplay(); loading.value = false; readFailed.value = false;
  }, { flush: "sync" });
  async function refresh() {
    const scope = selected();
    if (disposed || !scope || !scope.scanId || document.hidden || loading.value) return;
    const request = ++generation, requestedKey = key();
    const current = () => !disposed && request === generation && key() === requestedKey;
    loading.value = true;
    try {
      // Bounded work per wakeup. Continuation uses the SAME lifecycle scheduler,
      // rather than spawning unbounded timers or one listener per process.
      for (let pages = 0; pages < 4 && current() && !document.hidden; pages++) {
        const prior = replay.value;
        const page = await sentinelApi.readNativeProcessLog(scope.scanId, scope.attempt, prior.attempt, prior.cursor, 300);
        if (!current()) return;
        replay.value = acceptNativePage(page, scope, prior);
        readFailed.value = false;
        if (!replay.value.more) break;
      }
    } catch { if (current()) readFailed.value = true; }
    finally {
      if (current()) {
        loading.value = false;
        if (replay.value.more && !readFailed.value) committed.catchUp();
      }
    }
  }
  const committed = useCommittedRefresh("nest-native-process-log", key,
    payload => acceptsNativeHint(payload, selected()), () => loading.value, refresh,
    unavailable => { connectionUnavailable.value = unavailable; });
  onBeforeUnmount(() => { disposed = true; ++generation; });
  return { replay, loading, readFailed, connectionUnavailable, refresh };
}
