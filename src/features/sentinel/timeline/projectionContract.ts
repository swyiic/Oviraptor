import type { AgentTimelineItem } from "../../../types";
import { chatDecisionDisplay } from "./rootDecisionContract";

// Identity is event-scoped. Delimiters can occur in either field, so all
// cache, reading and rendering consumers must encode the tuple losslessly.
export function timelineIdentity(item: Pick<AgentTimelineItem, "eventType" | "id">): string {
  return JSON.stringify([item.eventType, item.id]);
}

const optionalTextFields = [
  "threadKey", "targetKey", "correlationId", "assignmentId", "fromRole",
  "toRole", "fromRunId", "toRunId", "messageKind", "deliveryState", "ackState", "status",
] as const;

// Shared by live snapshots and historical pages. This checks the message
// envelope consumed by rendering/reading, not nested receipts or authorization.
// Timestamp order and event-scoped IDs remain valid; timestamps need not be ISO.
export function validTimelineRows(items: AgentTimelineItem[]): boolean {
  if (!Array.isArray(items)) return false;
  const sequences = new Set<number>();
  const identities = new Set<string>();
  for (const item of items) {
    if (!item || typeof item !== "object" || Array.isArray(item)
      || typeof item.id !== "string" || !item.id.trim()
      || typeof item.eventType !== "string" || !item.eventType.trim()
      || !Number.isSafeInteger(item.sequence) || item.sequence <= 0
      || typeof item.timestamp !== "string" || typeof item.summary !== "string"
      || optionalTextFields.some((field) => item[field] != null && typeof item[field] !== "string")) return false;
    for (const values of [item.reasonCodes, item.requiredApprovals]) {
      if (values != null && (!Array.isArray(values) || values.some((value) => typeof value !== "string"))) return false;
    }
    if (item.eventType === "root_decision" && !chatDecisionDisplay(item)) return false;
    const identity = timelineIdentity(item);
    if (sequences.has(item.sequence) || identities.has(identity)) return false;
    sequences.add(item.sequence);
    identities.add(identity);
  }
  return true;
}
