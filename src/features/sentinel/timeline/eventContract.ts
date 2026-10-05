// Shared display hint contract. Only a subsequent validated DB read may publish
// content; event payloads never grant execution authority or replace snapshots.
export interface CollaborationEventNotification {
  sequence: number;
  scanId: string;
  attemptNumber: number;
}

export function validCollaborationEvent(value: unknown): value is CollaborationEventNotification {
  if (!value || typeof value !== "object") return false;
  const item = value as Partial<CollaborationEventNotification>;
  return typeof item.scanId === "string" && item.scanId.length > 0
    && Number.isSafeInteger(item.sequence) && (item.sequence ?? 0) > 0
    && Number.isSafeInteger(item.attemptNumber) && (item.attemptNumber ?? 0) > 0;
}
