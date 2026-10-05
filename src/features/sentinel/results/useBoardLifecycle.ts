import { onMounted, onUnmounted } from "vue";

interface BoardLifecycleOptions {
  subscribe: () => Promise<() => void>;
  load: () => Promise<void>;
  restoreSearch: () => Promise<void>;
  refresh: () => Promise<void>;
  onSubscriptionError: (message: string) => void;
}

// Own only this view's subscription and polling lifetime. A slow desktop event
// bridge must not prevent reading existing local task data.
export function useBoardLifecycle(options: BoardLifecycleOptions) {
  let disposed = false;
  let unsubscribe: (() => void) | undefined;
  let timer: number | undefined;

  async function subscribe() {
    try {
      const release = await options.subscribe();
      if (disposed) release();
      else unsubscribe = release;
    } catch {
      if (!disposed) {
        options.onSubscriptionError("身份状态通知连接失败，本地任务数据仍可查看");
      }
    }
  }

  onMounted(async () => {
    void subscribe();
    await options.load();
    if (disposed) return;
    await options.restoreSearch();
    if (disposed) return;
    timer = window.setInterval(() => {
      if (!disposed) void options.refresh();
    }, 12000);
  });

  onUnmounted(() => {
    disposed = true;
    if (timer !== undefined) window.clearInterval(timer);
    timer = undefined;
    const release = unsubscribe;
    unsubscribe = undefined;
    release?.();
  });
}
