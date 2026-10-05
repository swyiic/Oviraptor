import { onMounted, onUnmounted } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { validCollaborationEvent } from "../timeline/eventContract";

export function useLiveTrace(
  initialLoad: () => Promise<void>,
  refresh: (eventDriven?: boolean) => Promise<void>,
  selectedId: () => string,
  onConnectionError: () => void,
) {
  let timer: number | undefined;
  let coalesced: number | undefined;
  let unlisten: UnlistenFn | undefined;
  let mounted = false;
  let ready = false;
  let inFlight = false;
  let subscribing = false;
  let connectionWarningShown = false;
  let pendingTask = "";

  function schedule() {
    if (!mounted || !ready || document.hidden || inFlight || coalesced !== undefined || !pendingTask) return;
    coalesced = window.setTimeout(() => {
      coalesced = undefined;
      void reconcile(true);
    }, 50);
  }

  async function reconcile(eventDriven = false) {
    if (!mounted || !ready || document.hidden || inFlight) return;
    if (eventDriven && pendingTask !== selectedId()) { pendingTask = ""; return; }
    pendingTask = "";
    inFlight = true;
    try {
      // Both event hints and the lost-notification fallback must read committed
      // state, including terminal tasks and a previously failed initial read.
      // Completion does not prove that the UI received the final stored rows.
      await refresh(true);
    } catch {
      // Retain the last verified view; the next hint/reconciliation retries.
    } finally {
      inFlight = false;
      schedule();
    }
  }

  function catchUp() {
    if (!mounted) return;
    pendingTask = selectedId();
    schedule();
  }

  async function subscribe() {
    if (!mounted || unlisten || subscribing) return;
    subscribing = true;
    try {
      const release = await listen<unknown>("nest://collaboration-event", ({ payload }) => {
        if (!mounted || !validCollaborationEvent(payload) || payload.scanId !== selectedId()) return;
        // Read the current DB view, never the event's content or attempt state.
        catchUp();
      });
      if (!mounted) release();
      else {
        unlisten = release;
        connectionWarningShown = false;
        // Close the gap if registration completed after the initial DB read.
        if (ready) catchUp();
      }
    } catch {
      if (mounted && !connectionWarningShown) {
        connectionWarningShown = true;
        onConnectionError();
      }
    } finally { subscribing = false; }
  }

  function onVisibility() {
    if (!document.hidden) { void subscribe(); catchUp(); }
  }

  onMounted(async () => {
    mounted = true;
    document.addEventListener("visibilitychange", onVisibility);
    void subscribe();
    await initialLoad();
    if (!mounted) return;
    ready = true;
    schedule();
    // Low-frequency fallback for lost notifications; not the primary live path.
    timer = window.setInterval(() => {
      if (document.hidden) return;
      void subscribe();
      void reconcile(Boolean(pendingTask));
    }, 15000);
  });
  onUnmounted(() => {
    mounted = false;
    ready = false;
    pendingTask = "";
    if (timer !== undefined) window.clearInterval(timer);
    if (coalesced !== undefined) window.clearTimeout(coalesced);
    document.removeEventListener("visibilitychange", onVisibility);
    unlisten?.();
    unlisten = undefined;
  });
}
