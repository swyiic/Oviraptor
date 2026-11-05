import { ref } from "vue";

/** Text the trace "+" menu asks the scan-agent composer to insert. */
export const steeringInsert = ref("");

export function queueSteeringInsert(text: string) {
  const next = text.trim();
  if (!next) return;
  steeringInsert.value = next;
}
