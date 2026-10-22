import type { AgentHostConversation } from './chatClient';

export function conversationActivityAt(item: Pick<AgentHostConversation, 'createdAt' | 'lastActivityAt' | 'updatedAt'>): string | undefined {
  const value = item.lastActivityAt || item.updatedAt || item.createdAt;
  return value && Number.isFinite(Date.parse(value)) ? value : undefined;
}

/** API creation order remains stable; display order is local and pins are durable metadata. */
export function sortConversationHistory(items: AgentHostConversation[]): AgentHostConversation[] {
  return items.map((item, index) => ({ item, index })).sort((a, b) =>
    Number(Boolean(b.item.pinned)) - Number(Boolean(a.item.pinned))
    || (Date.parse(conversationActivityAt(b.item) || '') || 0) - (Date.parse(conversationActivityAt(a.item) || '') || 0)
    || b.index - a.index).map(entry => entry.item);
}

export function conversationHistoryTime(value: string | undefined, language = 'zh', now = new Date()): { short: string; full: string } {
  if (!value) return { short: '', full: '' };
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return { short: '', full: '' };
  const locale = language === 'en' ? 'en-GB' : 'zh-CN';
  const full = date.toLocaleString(locale, { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });
  const day = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const yesterday = new Date(day); yesterday.setDate(day.getDate() - 1);
  const time = date.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit', hour12: false });
  const short = date >= day && date <= now ? time
    : date >= yesterday && date < day ? `${language === 'en' ? 'Yesterday' : '昨天'} ${time}`
      : date.toLocaleDateString(locale, { ...(date.getFullYear() !== now.getFullYear() ? { year: 'numeric' as const } : {}), month: '2-digit', day: '2-digit' });
  return { short, full };
}
