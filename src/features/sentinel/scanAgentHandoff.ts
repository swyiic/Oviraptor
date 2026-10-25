import type { SentinelScan } from './types';
import type { DiscoveryChatRequest } from '../assets/discoveryAnalysis';

/** Desktop project identity. Host project mapping remains owned by the chat client. */
export interface ScanAgentHandoff { projectId: number; taskId: string; attemptNumber: number }

function valid(value: ScanAgentHandoff): boolean {
  return !!value && Number.isSafeInteger(value.projectId) && value.projectId > 0
    && typeof value.taskId === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value.taskId)
    && Number.isSafeInteger(value.attemptNumber) && value.attemptNumber >= 0;
}

export function scanAgentHandoff(projectId: number | undefined, scan?: SentinelScan): ScanAgentHandoff | undefined {
  if (!scan || projectId === undefined || scan.projectId !== projectId) return;
  const value = { projectId, taskId: scan.id, attemptNumber: scan.latestAttemptNumber };
  return valid(value) ? value : undefined;
}

/** Shell revalidates after navigation; a stale child event must not open another project's chat. */
export function scanAgentChatRequest(projectId: number | undefined, id: number, value: ScanAgentHandoff): DiscoveryChatRequest | undefined {
  if (!valid(value) || value.projectId !== projectId || !Number.isSafeInteger(id) || id < 1) return;
  return { id, projectId: value.projectId, prompt: scanAgentPrompt(value) };
}

/** Deliberately passes identifiers only: no target URLs, credentials, logs or local paths. */
export function scanAgentPrompt(value: ScanAgentHandoff): string {
  return `请只读核对当前项目的 Nest 扫描任务 taskId=${value.taskId}。打开工作台时看到的是第 ${value.attemptNumber} 次尝试，这只是界面快照，请以工具读取的最新记录为准。\n\n先用 scan_status 核对同一 taskId，再用 scan_results 读取目标状态、调查决策、运行时诊断及其选定证据字段；按原 taskId 和筛选条件逐页读取，空页仍有游标时读取下一页。仅使用本轮实际列出的可用工具，不猜工具名或记录 ID。\n\n请告诉我：实际完成了什么、哪些目标尚未完成、停止原因及下一步。区分历史记录、独立复核与本次实际执行。不要新建、启动、暂停、恢复或重新执行任务，不改变范围、预算或权限，不重复付费请求。若后续需要执行，我会在此对话中明确提出。`;
}
