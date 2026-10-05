import { useCommittedRefresh } from "../../../composables/useCommittedRefresh";

// attempt=0 follows the latest committed attempt; positive values pin a round.
export type LogScope = { scanId: string; attempt: number };

export function useLiveRunnerLog(
  selected: () => LogScope | undefined,
  busy: () => boolean,
  refresh: (scope: LogScope) => Promise<void>,
  connectionState: (unavailable: boolean) => void,
) {
  useCommittedRefresh("nest-runner-log", () => {
    const scope = selected();
    return scope ? JSON.stringify([scope.scanId, scope.attempt]) : "";
  }, payload => {
    if (!payload || typeof payload !== "object") return false;
    const hint = payload as Partial<LogScope>, scope = selected();
    return Boolean(scope && hint.scanId === scope.scanId && Number.isSafeInteger(hint.attempt)
      && (hint.attempt ?? 0) > 0 && (scope.attempt === 0 || hint.attempt === scope.attempt));
  }, busy, async () => {
    const scope = selected();
    if (scope) await refresh(scope);
  }, connectionState);
}
