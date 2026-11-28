// `weflow serve` 的见解（#210），打开页面时读取：一次读一页，可以只看一个会话或按关键词筛选；
// 换了服务端或账号、或者见解有了变化时重读。
import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiError } from '../api/client';
import { t } from '../i18n';
import { getInsights, PAGE, type InsightPage } from '../api/insights';

/** 见解的筛选条件：关键词，以及一个会话或者全部。 */
export interface InsightFilter { keyword: string; session: string | null }

/** 见解的读取状态：还没读或正在读；读到的一页和是否还有更多；或者失败的原因。 */
export type InsightsState =
  | { status: 'idle' | 'loading' }
  | { status: 'ready'; page: InsightPage; more: boolean; loadingMore: boolean; error?: string }
  | { status: 'error'; message: string };

const messageOf = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || fallback;

/** `enabled` 之后按 `filter` 筛选的见解；`scope` 或筛选条件变化时重读。 */
export function useInsights(enabled: boolean, scope: number, filter: InsightFilter) {
  const [state, setState] = useState<InsightsState>({ status: 'idle' });
  const [round, setRound] = useState(0);
  const reload = useCallback(() => setRound(n => n + 1), []);
  const latest = useRef(state);
  latest.current = state;
  // 已显示的页所属的筛选条件，以及读取它下一页的请求：请求或显示另一个第一页时取消这次读取，
  // 免得旧筛选条件的见解加到新的列表里
  const shown = useRef(filter);
  const reading = useRef<AbortController | null>(null);
  const stopMore = () => { reading.current?.abort(); reading.current = null; };
  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    // 已经显示了一页时，读取期间（换了筛选条件或原地重读）继续显示它，读到后再换成新的第一页
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

  /** 立即在页面上把一条见解标为已读；通知服务端由调用方负责。 */
  const markRead = useCallback((id: string) => {
    const now = latest.current;
    if (now.status !== 'ready') return;
    const found = now.page.records.find(r => r.id === id);
    if (!found || found.read) return;
    setState({ ...now, page: { ...now.page, unreadCount: Math.max(0, now.page.unreadCount - 1), records: now.page.records.map(r => (r.id === id ? { ...r, read: true } : r)) } });
  }, []);

  return { state, reload, more, markRead };
}
