// Shared task-navigation envelope, not a full SentinelScan schema or execution
// authorization check. Legacy statuses/timestamps may be empty; global views
// may contain unassigned tasks and rows from different projects.
export function validTaskPage(value: unknown, limit: number, projectId?: number): boolean {
  if (!Array.isArray(value) || value.length > limit) return false;
  const ids = new Set<string>();
  return value.every((row: unknown) => {
    if (!row || typeof row !== "object" || Array.isArray(row)) return false;
    const task = row as Record<string, unknown>;
    if (typeof task.id !== "string" || !task.id.trim() || task.id.length > 256
      || ids.has(task.id) || typeof task.updatedAt !== "string" || typeof task.status !== "string"
      || (projectId !== undefined && task.projectId !== projectId)
      || (task.projectId != null && (!Number.isSafeInteger(task.projectId) || (task.projectId as number) < 1)))
      return false;
    ids.add(task.id);
    return true;
  });
}
