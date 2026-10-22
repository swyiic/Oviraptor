import { Worker } from 'node:worker_threads';

/** One serial SQLite worker per Host, with identical concurrent reads coalesced.
 * No TTL: the worker checks live source and mirror revisions for every read. */
export class DesktopAssetSynchronizer {
  constructor() { this.worker = null; this.pending = new Map(); this.inflight = new Map(); this.nextId = 0; this.closed = false; }
  sync(input) {
    if (this.closed) return Promise.reject(Error('desktop_synchronizer_closed'));
    const key = JSON.stringify(input);
    if (this.inflight.has(key)) return this.inflight.get(key);
    if (!this.worker) {
      const worker = new Worker(new URL('./desktop-asset-sync-worker.mjs', import.meta.url), { execArgv: [] });
      this.worker = worker;
      worker.on('message', ({ id, result }) => {
        const waiting = this.pending.get(id); if (!waiting) return;
        this.pending.delete(id); waiting.resolve(result);
        if (!this.pending.size) worker.unref();
      });
      const failed = () => {
        if (this.worker !== worker) return;
        this.worker = null;
        for (const waiting of this.pending.values()) waiting.resolve({ synced: 0, reason: 'desktop_sync_failed' });
        this.pending.clear();
        void worker.terminate();
      };
      worker.on('error', failed); worker.on('exit', failed);
    }
    const promise = new Promise(resolve => {
      const id = ++this.nextId;
      this.pending.set(id, { resolve }); this.worker.ref();
      this.worker.postMessage({ id, input });
    }).finally(() => { if (this.inflight.get(key) === promise) this.inflight.delete(key); });
    this.inflight.set(key, promise);
    return promise;
  }
  async close() {
    this.closed = true;
    const worker = this.worker; this.worker = null;
    for (const waiting of this.pending.values()) waiting.resolve({ synced: 0, reason: 'desktop_sync_failed' });
    this.pending.clear(); this.inflight.clear();
    if (worker) await worker.terminate();
  }
}
