// The AI insights (#210): the records the insight engine of `serve --insight` (or `insight trigger`) wrote, through
// the command routes. The records of the current account only, newest first.
import { call, get } from './client';

/** A record as `insight records` lists it. */
export interface InsightRecord {
  id: string;
  /** Unix milliseconds. */
  createdAt: number;
  sessionId: string;
  displayName: string;
  avatarUrl?: string;
  /** Why it was written: `activity`, `silence`, `test`, … */
  triggerReason: string;
  insight: string;
  read: boolean;
}

/** How a record came to be: the request to the model and its answer (`insight get`). */
export interface InsightLog {
  endpoint: string;
  model: string;
  maxTokens: number;
  triggerReason: string;
  contextCount: number;
  systemPrompt: string;
  userPrompt: string;
  rawOutput: string;
  durationMs: number;
}

export interface InsightContact { sessionId: string; displayName: string; avatarUrl?: string; count: number }

export interface InsightPage {
  records: InsightRecord[];
  total: number;
  todayCount: number;
  unreadCount: number;
  /** Everyone with a record, the most records first. */
  contacts: InsightContact[];
}

export const PAGE = 50;

export async function getInsights(filter: { keyword?: string; session?: string | null; offset?: number }, signal?: AbortSignal): Promise<InsightPage> {
  const page = await get<Partial<InsightPage>>('insight/records', {
    keyword: filter.keyword?.trim() || undefined, session: filter.session || undefined, limit: PAGE, offset: filter.offset || undefined,
  }, signal);
  return { records: page.records ?? [], total: page.total ?? 0, todayCount: page.todayCount ?? 0, unreadCount: page.unreadCount ?? 0, contacts: page.contacts ?? [] };
}

export async function getInsightLog(id: string, signal?: AbortSignal): Promise<InsightLog | null> {
  const found = await get<{ record?: { log?: InsightLog } }>('insight/get', { id }, signal);
  return found.record?.log ?? null;
}

export const markInsightRead = (id: string) => call<unknown>('PUT', 'insight/mark-read', { id });

/** `insight clear`: every record, or one conversation's. */
export const clearInsights = (session?: string | null) => call<{ removed?: number }>('DELETE', 'insight/clear', { session: session || undefined });

/** `insight trigger [session]`: one insight now, for that conversation or the first one it may write for. */
export const triggerInsight = (session?: string | null) =>
  call<{ message?: string; record?: InsightRecord }>('POST', 'insight/trigger', { 'session-id': session || undefined });

export const REASONS: Record<string, string> = { activity: '聊天活跃', silence: '久未联系', test: '手动生成' };
export const reasonLabel = (reason: string) => REASONS[reason] ?? reason;
