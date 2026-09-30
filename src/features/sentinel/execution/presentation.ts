// Historical backend values are provenance for display only. This module must
// never participate in backend selection, recovery, or execution admission.
// REM-009 Loop1: display is neutral ("历史封存（只读）"); the "strix" condition
// below is old-data compat only (retired backend value in existing rows),
// pending DB cleanup. Do not reintroduce brand-specific display strings.
export function agentBackendLabel(backend?: string): string {
  if (backend === "native") return "原生 Agent";
  if (backend === "legacy_backend_removed" || backend === "strix") {
    return "历史封存（只读）";
  }
  return "未记录";
}

export function agentModeLabel(mode?: string): string {
  return ({ quick: "快速", standard: "标准", deep: "深度" } as Record<string, string>)[mode || ""]
    || mode || "未记录";
}
