// Display labels cannot grant backend selection, recovery, or execution admission.
export function agentBackendLabel(backend?: string): string {
  if (backend === "native") return "原生 Agent";
  if (backend === "legacy_backend_removed") {
    return "历史封存（只读）";
  }
  return "未记录";
}

export function agentModeLabel(mode?: string): string {
  return ({ quick: "快速", standard: "标准", deep: "深度" } as Record<string, string>)[mode || ""]
    || mode || "未记录";
}
