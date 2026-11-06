import { useI18n } from "../../../i18n";

const { tr } = useI18n();

// Display labels cannot grant backend selection, recovery, or execution admission.
export function agentBackendLabel(backend?: string): string {
  if (backend === "native") return tr("原生 Agent", "Native Agent");
  if (backend === "legacy_backend_removed") {
    return tr("历史封存（只读）", "Archived legacy (read-only)");
  }
  return tr("未记录", "Not recorded");
}

export function agentModeLabel(mode?: string): string {
  return ({ quick: tr("快速", "Quick"), standard: tr("标准", "Standard"), deep: tr("深度", "Deep") } as Record<string, string>)[mode || ""]
    || mode || tr("未记录", "Not recorded");
}
