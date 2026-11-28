// 通过 `weflow serve` 搜索消息（#199）：先读一页结果，需要时再读更多；新的搜索取消旧的。
import { useCallback, useRef, useState } from 'react';
import { ApiError } from '../api/client';
import { SEARCH_PAGE, searchMessages, type SearchHit, type SearchQuery } from '../api/messages';
import { t } from '../i18n';

/** 一次搜索：查询条件、目前读到的结果和读取的状态。 */
export interface SearchState {
  status: 'idle' | 'loading' | 'ready' | 'error';
  query: SearchQuery | null;
  hits: SearchHit[];
  hasMore: boolean;
  loadingMore: boolean;
  error?: string;
}

const IDLE: SearchState = { status: 'idle', query: null, hits: [], hasMore: false, loadingMore: false };

const messageOf = (problem: unknown) => (problem instanceof ApiError && problem.message) || t('搜索失败，请重试。');

/** 搜索的状态，以及操作它的函数：开始新的查询、读下一页结果、清空。 */
export function useSearch(): { state: SearchState; search(query: SearchQuery): void; more(): void; clear(): void } {
  const [state, setState] = useState<SearchState>(IDLE);
  const latest = useRef(state);
  latest.current = state;
  const running = useRef<AbortController | null>(null);

  const start = () => {
    running.current?.abort();
    const controller = new AbortController();
    running.current = controller;
    return controller;
  };

  const search = useCallback((query: SearchQuery) => {
    const controller = start();
    setState({ ...IDLE, status: 'loading', query });
    searchMessages(query, 0, controller.signal).then(hits => {
      if (!controller.signal.aborted) setState({ ...IDLE, status: 'ready', query, hits, hasMore: hits.length === SEARCH_PAGE });
    }, problem => {
      if (!controller.signal.aborted) setState({ ...IDLE, status: 'error', query, error: messageOf(problem) });
    });
  }, []);

  const more = useCallback(() => {
    const now = latest.current;
    if (now.status !== 'ready' || !now.hasMore || now.loadingMore || !now.query) return;
    const controller = start();
    const query = now.query;
    setState({ ...now, loadingMore: true, error: undefined });
    searchMessages(query, now.hits.length, controller.signal).then(hits => {
      if (controller.signal.aborted) return;
      setState(old => {
        const seen = new Set(old.hits.map(h => h._session_id + ' ' + h._cursor));
        return { ...old, loadingMore: false, hasMore: hits.length === SEARCH_PAGE, hits: [...old.hits, ...hits.filter(h => !seen.has(h._session_id + ' ' + h._cursor))] };
      });
    }, problem => {
      if (!controller.signal.aborted) setState(old => ({ ...old, loadingMore: false, error: messageOf(problem) }));
    });
  }, []);

  const clear = useCallback(() => {
    running.current?.abort();
    setState(IDLE);
  }, []);

  return { state, search, more, clear };
}
