// The insights of `weflow serve` (#210), read once the page opens: a page at a time, narrowed to a conversation or to
// words, read again for another server or account and when something changed them.
import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiError } from '../api/client';
import { t } from '../i18n';
import { getInsights, PAGE, type InsightPage } from '../api/insights';

export interface InsightFilter { keyword: string; session: string | null }

export type InsightsState =
  | { status: 'idle' | 'loading' }
  | { status: 'ready'; page: InsightPage; more: boolean; loadingMore: boolean; error?: string }
  | { status: 'error'; message: string };

const messageOf = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || fallback;

export function useInsights(enabled: boolean, scope: number, filter: InsightFilter) {
  const [state, setState] = useState<InsightsState>({ status: 'idle' });
  const [round, setRound] = useState(0);
  const reload = useCallback(() => setRound(n => n + 1), []);
  const latest = useRef(state);
  latest.current = state;
  // the filter of the page shown, and the read of its next page: aborted once another first page is asked for or shown,
  // so that the records of a filter left are not added to those of the next
  const shown = useRef(filter);
  const reading = useRef<AbortController | null>(null);
  const stopMore = () => { reading.current?.abort(); reading.current = null; };
  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    // a new filter shows its first page; reading again in place keeps the page on screen meanwhile
    if (latest.current.status !== 'ready') setState({ status: 'loading' });
    getInsights(filter, controller.signal).then(page => {
      if (controller.signal.aborted) return;
      stopMore();
      shown.current = filter;
      setState({ status: 'ready', page, more: page.records.length < page.total, loadingMore: false });
    }, problem => { if (!controller.signal.aborted) setState({ status: 'error', message: messageOf(problem, t('读取见解失败')) }); });
    return () => { controller.abort(); stopMore(); };
  }, [enabled, scope, filter, round]);

  const more = useCallback(() => {
    const now = latest.current;
    if (now.status !== 'ready' || !now.more || now.loadingMore) return;
    stopMore();
    const controller = new AbortController();
    reading.current = controller;
    const { signal } = controller;
    setState({ ...now, loadingMore: true });
    getInsights({ ...shown.current, offset: now.page.records.length }, signal).then(next => {
      const cur = latest.current;
      if (signal.aborted || cur.status !== 'ready') return;
      const records = [...cur.page.records, ...next.records];
      setState({ ...cur, page: { ...next, records }, more: next.records.length === PAGE && records.length < next.total, loadingMore: false });
    }, problem => {
      const cur = latest.current;
      if (!signal.aborted && cur.status === 'ready') setState({ ...cur, loadingMore: false, error: messageOf(problem, t('读取更早的见解失败')) });
    });
  }, []);

  /** Marks a record read on the page at once; the server is told by the caller. */
  const markRead = useCallback((id: string) => {
    const now = latest.current;
    if (now.status !== 'ready') return;
    const found = now.page.records.find(r => r.id === id);
    if (!found || found.read) return;
    setState({ ...now, page: { ...now.page, unreadCount: Math.max(0, now.page.unreadCount - 1), records: now.page.records.map(r => (r.id === id ? { ...r, read: true } : r)) } });
  }, []);

  return { state, reload, more, markRead };
}
