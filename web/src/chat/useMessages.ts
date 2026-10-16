import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiError } from '../api/client';
import { getMessages, PAGE_SIZE, type PageFrom, type ParsedMessage } from '../api/messages';

/** A message to bring into view after a jump: a search hit in the middle, a day from its first message at the top. */
export interface Focus {
  cursor: string;
  align: 'center' | 'start';
  /** Tells two jumps to the same message apart. */
  seq: number;
}

export interface Thread {
  status: 'idle' | 'loading' | 'ready' | 'error';
  /** Oldest first, as they are shown. */
  messages: ParsedMessage[];
  /** Whether older messages are left to load. */
  hasMore: boolean;
  loadingOlder: boolean;
  /** After a jump into the past: whether newer messages are left to load below (#199). */
  hasNewer: boolean;
  loadingNewer: boolean;
  focus: Focus | null;
  error?: string;
}

/** Where to show a conversation from: a time (a day picked in the calendar) or a message (a search hit). */
export type Target = { at: 'time'; seconds: number } | { at: 'message'; message: ParsedMessage };

export interface Jump {
  sessionId: string;
  target: Target;
  seq: number;
}

const IDLE: Thread = { status: 'idle', messages: [], hasMore: false, loadingOlder: false, hasNewer: false, loadingNewer: false, focus: null };
const LOADING: Thread = { ...IDLE, status: 'loading' };
/** Messages read on each side of a search hit. */
const AROUND = 25;

/**
 * Conversations read in this tab, so going back to one shows it at once (#159); the oldest are dropped. Only threads
 * that reach the newest message are kept: one left in the past by a jump opens at the newest again.
 */
const cache = new Map<string, Thread>();
const CACHE_SIZE = 20;

function remember(id: string, thread: Thread) {
  if (thread.hasNewer) return;
  cache.delete(id);
  cache.set(id, { ...thread, loadingOlder: false, loadingNewer: false, focus: null });
  while (cache.size > CACHE_SIZE) cache.delete(cache.keys().next().value!);
}

/** Forgets every conversation read, as after the account changed. */
export function forgetMessages() {
  cache.clear();
}

function messageOf(problem: unknown): string {
  if (problem instanceof ApiError) {
    // A server without `--parsed` (rc.4 and older) or `--since` refuses the parameter before reading anything.
    if (problem.status === 400 && problem.code === 'invalid_arguments') {
      return '这个版本的 weflow serve 还不能按类型返回消息或按日期跳转，请更新到支持 chat messages --parsed 和 --since 的版本。';
    }
    return problem.message || '读取消息失败';
  }
  return '读取消息失败';
}

/** The thread around a target: the page of a day, or the messages on both sides of a hit. */
async function threadAt(sessionId: string, target: Target, seq: number, signal: AbortSignal): Promise<Thread> {
  if (target.at === 'message') {
    const hit = target.message;
    const [older, newer] = await Promise.all([
      getMessages(sessionId, { before: hit._cursor }, signal, AROUND),
      getMessages(sessionId, { after: hit._cursor }, signal, AROUND),
    ]);
    return {
      ...IDLE, status: 'ready', messages: [...older.reverse(), hit, ...newer.reverse()],
      hasMore: older.length === AROUND, hasNewer: newer.length === AROUND, focus: { cursor: hit._cursor, align: 'center', seq },
    };
  }
  const page = await getMessages(sessionId, { since: target.seconds }, signal);
  if (page.length) {
    const messages = page.slice().reverse();
    return { ...IDLE, status: 'ready', messages, hasMore: true, hasNewer: page.length === PAGE_SIZE, focus: { cursor: messages[0]._cursor, align: 'start', seq } };
  }
  // nothing from that day on: the newest messages, the last one in view
  const latest = await getMessages(sessionId, null, signal);
  const messages = latest.slice().reverse();
  return { ...IDLE, status: 'ready', messages, hasMore: latest.length === PAGE_SIZE, focus: messages.length ? { cursor: messages[messages.length - 1]._cursor, align: 'center', seq } : null };
}

/**
 * The messages of a conversation: its newest page first, older pages on demand; after a jump (a day, a search
 * hit) the page there, with older pages above and newer ones below on demand. The state remembers whose it is, so
 * right after a switch the new conversation never shows (or is scrolled as) the previous one's messages.
 */
export function useMessages(sessionId: string | null, jump: Jump | null = null): {
  thread: Thread; loadOlder(): void; loadNewer(): void; reload(): void;
} {
  const [owned, setOwned] = useState<{ id: string | null; thread: Thread }>(() => ({ id: sessionId, thread: (sessionId && cache.get(sessionId)) || (sessionId ? LOADING : IDLE) }));
  const pendingJump = jump && jump.sessionId === sessionId && jump.seq !== owned.thread.focus?.seq;
  const thread = owned.id === sessionId ? owned.thread : pendingJump ? LOADING : (sessionId && cache.get(sessionId)) || (sessionId ? LOADING : IDLE);
  const latest = useRef(thread);
  latest.current = thread;
  const current = useRef(sessionId);
  const paging = useRef<AbortController | null>(null);
  const fresh = useRef(false);
  const handled = useRef<number | null>(null);
  const [round, setRound] = useState(0);

  const update = useCallback((id: string | null, change: Thread | ((old: Thread) => Thread)) => {
    setOwned(old => {
      if (typeof change !== 'function') return { id, thread: change };
      return old.id === id ? { id, thread: change(old.thread) } : old;
    });
  }, []);

  useEffect(() => {
    current.current = sessionId;
    paging.current?.abort();
    if (!sessionId) { update(null, IDLE); return; }
    const target = jump && jump.sessionId === sessionId && jump.seq !== handled.current ? jump : null;
    const controller = new AbortController();
    if (target) {
      handled.current = target.seq;
      update(sessionId, LOADING);
      threadAt(sessionId, target.target, target.seq, controller.signal).then(next => {
        remember(sessionId, next);
        update(sessionId, next);
      }, problem => {
        if (!controller.signal.aborted) update(sessionId, { ...IDLE, status: 'error', error: messageOf(problem) });
      });
      return () => controller.abort();
    }
    const kept = fresh.current ? undefined : cache.get(sessionId);
    fresh.current = false;
    if (kept) { update(sessionId, kept); return; }
    update(sessionId, LOADING);
    getMessages(sessionId, null, controller.signal).then(page => {
      const next: Thread = { ...IDLE, status: 'ready', messages: page.slice().reverse(), hasMore: page.length === PAGE_SIZE };
      remember(sessionId, next);
      update(sessionId, next);
    }, problem => {
      if (!controller.signal.aborted) update(sessionId, { ...IDLE, status: 'error', error: messageOf(problem) });
    });
    return () => controller.abort();
  }, [sessionId, round, jump, update]);

  /** The next page on one side: older above the first message, or newer below the last. */
  const page = useCallback((side: 'older' | 'newer') => {
    const id = current.current;
    const now = latest.current;
    const busy = now.loadingOlder || now.loadingNewer;
    const left = side === 'older' ? now.hasMore : now.hasNewer;
    if (!id || now.status !== 'ready' || !left || busy || !now.messages.length) return;
    const controller = new AbortController();
    paging.current = controller;
    const flag = side === 'older' ? 'loadingOlder' : 'loadingNewer';
    update(id, { ...now, [flag]: true, error: undefined });
    const from: PageFrom = side === 'older' ? { before: now.messages[0]._cursor } : { after: now.messages[now.messages.length - 1]._cursor };
    getMessages(id, from, controller.signal).then(got => {
      if (controller.signal.aborted || current.current !== id) return;
      update(id, old => {
        const seen = new Set(old.messages.map(m => m._cursor));
        const added = got.filter(m => !seen.has(m._cursor)).reverse();
        const next: Thread = side === 'older'
          ? { ...old, loadingOlder: false, hasMore: got.length === PAGE_SIZE, messages: [...added, ...old.messages] }
          : { ...old, loadingNewer: false, hasNewer: got.length === PAGE_SIZE, messages: [...old.messages, ...added] };
        remember(id, next);
        return next;
      });
    }, problem => {
      if (!controller.signal.aborted && current.current === id) update(id, old => ({ ...old, [flag]: false, error: messageOf(problem) }));
    });
  }, [update]);

  const loadOlder = useCallback(() => page('older'), [page]);
  const loadNewer = useCallback(() => page('newer'), [page]);

  /** Reads the newest page again (after an error, or to leave a jump for the newest messages). */
  const reload = useCallback(() => {
    fresh.current = true;
    setRound(n => n + 1);
  }, []);

  return { thread, loadOlder, loadNewer, reload };
}
