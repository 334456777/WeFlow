import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiError } from '../api/client';
import { getMessages, PAGE_SIZE, type ParsedMessage } from '../api/messages';

export interface Thread {
  status: 'idle' | 'loading' | 'ready' | 'error';
  /** Oldest first, as they are shown. */
  messages: ParsedMessage[];
  /** Whether older messages are left to load. */
  hasMore: boolean;
  loadingOlder: boolean;
  error?: string;
}

const IDLE: Thread = { status: 'idle', messages: [], hasMore: false, loadingOlder: false };
const LOADING: Thread = { ...IDLE, status: 'loading' };

/** Conversations read in this tab, so going back to one shows it at once (#159); the oldest are dropped. */
const cache = new Map<string, Thread>();
const CACHE_SIZE = 20;

function remember(id: string, thread: Thread) {
  cache.delete(id);
  cache.set(id, { ...thread, loadingOlder: false });
  while (cache.size > CACHE_SIZE) cache.delete(cache.keys().next().value!);
}

/** Forgets every conversation read, as after the account changed. */
export function forgetMessages() {
  cache.clear();
}

function messageOf(problem: unknown): string {
  if (problem instanceof ApiError) {
    // A server without `--parsed` (rc.4 and older) refuses the parameter before reading anything.
    if (problem.status === 400 && problem.code === 'invalid_arguments') {
      return '这个版本的 weflow serve 还不能按类型返回消息，请更新到支持 chat messages --parsed 的版本。';
    }
    return problem.message || '读取消息失败';
  }
  return '读取消息失败';
}

/**
 * The messages of a conversation: its newest page first, older pages on demand. The state remembers whose it is,
 * so right after a switch the new conversation never shows (or is scrolled as) the previous one's messages.
 */
export function useMessages(sessionId: string | null): { thread: Thread; loadOlder(): void; reload(): void } {
  const [owned, setOwned] = useState<{ id: string | null; thread: Thread }>(() => ({ id: sessionId, thread: (sessionId && cache.get(sessionId)) || (sessionId ? LOADING : IDLE) }));
  const thread = owned.id === sessionId ? owned.thread : (sessionId && cache.get(sessionId)) || (sessionId ? LOADING : IDLE);
  const latest = useRef(thread);
  latest.current = thread;
  const current = useRef(sessionId);
  const older = useRef<AbortController | null>(null);
  const fresh = useRef(false);
  const [round, setRound] = useState(0);

  const update = useCallback((id: string | null, change: Thread | ((old: Thread) => Thread)) => {
    setOwned(old => {
      if (typeof change !== 'function') return { id, thread: change };
      return old.id === id ? { id, thread: change(old.thread) } : old;
    });
  }, []);

  useEffect(() => {
    current.current = sessionId;
    older.current?.abort();
    if (!sessionId) { update(null, IDLE); return; }
    const kept = fresh.current ? undefined : cache.get(sessionId);
    fresh.current = false;
    if (kept) { update(sessionId, kept); return; }
    const controller = new AbortController();
    update(sessionId, LOADING);
    getMessages(sessionId, null, controller.signal).then(page => {
      const next: Thread = { status: 'ready', messages: page.slice().reverse(), hasMore: page.length === PAGE_SIZE, loadingOlder: false };
      remember(sessionId, next);
      update(sessionId, next);
    }, problem => {
      if (!controller.signal.aborted) update(sessionId, { ...IDLE, status: 'error', error: messageOf(problem) });
    });
    return () => controller.abort();
  }, [sessionId, round, update]);

  const loadOlder = useCallback(() => {
    const id = current.current;
    const now = latest.current;
    if (!id || now.status !== 'ready' || !now.hasMore || now.loadingOlder || !now.messages.length) return;
    const controller = new AbortController();
    older.current = controller;
    update(id, { ...now, loadingOlder: true, error: undefined });
    getMessages(id, now.messages[0]._cursor, controller.signal).then(page => {
      if (controller.signal.aborted || current.current !== id) return;
      update(id, old => {
        const seen = new Set(old.messages.map(m => m._cursor));
        const next: Thread = {
          ...old, loadingOlder: false, hasMore: page.length === PAGE_SIZE,
          messages: [...page.filter(m => !seen.has(m._cursor)).reverse(), ...old.messages],
        };
        remember(id, next);
        return next;
      });
    }, problem => {
      if (!controller.signal.aborted && current.current === id) update(id, old => ({ ...old, loadingOlder: false, error: messageOf(problem) }));
    });
  }, [update]);

  const reload = useCallback(() => {
    fresh.current = true;
    setRound(n => n + 1);
  }, []);

  return { thread, loadOlder, reload };
}
