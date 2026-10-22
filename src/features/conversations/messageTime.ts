/** Stored timestamps are UTC/offset ISO values. Never replace missing history with now. */
export function messageTimestamp(value: string | null | undefined, language = 'zh-CN') {
  if (typeof value !== 'string' || !value.trim()) return undefined;
  // Older SQLite UTC records may use a space and omit the explicit zone.
  const normalized = /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)?$/.test(value)
    ? value.replace(' ', 'T') + 'Z' : value;
  const date = new Date(normalized);
  if (!Number.isFinite(date.getTime())) return undefined;
  const options: Intl.DateTimeFormatOptions = { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' };
  return {
    iso: date.toISOString(),
    label: new Intl.DateTimeFormat(language, options).format(date),
    full: new Intl.DateTimeFormat(language, { ...options, second: '2-digit', timeZoneName: 'short' }).format(date),
  };
}
