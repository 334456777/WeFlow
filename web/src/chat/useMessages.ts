import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiError } from '../api/client';
import { onLive, useLive } from '../api/live';
import { currentScope, onNewScope } from '../api/scope';
import { getMessages, PAGE_SIZE, type PageFrom, type ParsedMessage } from '../api/messages';
import { t } from '../i18n';

/** 跳转后要显示的消息：搜索结果放在中间，某一天从它的第一条消息开始、放在顶部。 */
export interface Focus {
  cursor: string;
  align: 'center' | 'start';
  /** 区分跳到同一条消息的两次跳转。 */
  seq: number;
}

/** 一个会话目前读到的消息，以及读取的状态。 */
export interface Thread {
  status: 'idle' | 'loading' | 'ready' | 'error';
  /** 最早的在前，与显示的顺序一致。 */
  messages: ParsedMessage[];
  /** 是否还有更早的消息可以加载。 */
  hasMore: boolean;
  loadingOlder: boolean;
  /** 跳转到过去的位置后：下方是否还有更新的消息可以加载（#199）。 */
  hasNewer: boolean;
  loadingNewer: boolean;
  focus: Focus | null;
  error?: string;
}

/** 从哪里开始显示会话：一个时间（日历里选的一天）或一条消息（搜索结果）。 */
export type Target = { at: 'time'; seconds: number } | { at: 'message'; message: ParsedMessage };

/** 要在目标位置打开的会话，以及这是第几次跳转。 */
export interface Jump {
  sessionId: string;
  target: Target;
  seq: number;
}

const IDLE: Thread = { status: 'idle', messages: [], hasMore: false, loadingOlder: false, hasNewer: false, loadingNewer: false, focus: null };
const LOADING: Thread = { ...IDLE, status: 'loading' };
/** 搜索结果前后各读的消息数。 */
const AROUND = 25;

/**
 * 这个标签页里读过的会话，回到其中一个时立即显示（#159）；超过 `CACHE_SIZE` 个时丢掉最久没有保存过的。只保留读到了
 * 最新消息的会话：跳转后停在过去的会话，下次打开时重新从最新的消息开始。
 */
const cache = new Map<string, Thread>();
const CACHE_SIZE = 20;

/** 保存在 `scope` 里读到的会话；这期间开始了新的作用域，或者会话停在过去的位置时不保存。 */
function remember(id: string, thread: Thread, scope: number) {
  if (thread.hasNewer || scope !== currentScope()) return;
  cache.delete(id);
  cache.set(id, { ...thread, loadingOlder: false, loadingNewer: false, focus: null });
  while (cache.size > CACHE_SIZE) cache.delete(cache.keys().next().value!);
}

/** 清掉读过的全部会话，例如换了账号之后。 */
export function forgetMessages() {
  cache.clear();
}
onNewScope(forgetMessages);
// 缓存里的会话来了消息后，下次打开时重新读取；中断后任何会话都可能来过消息，全部清掉（#201）。
// 打开着的会话自己补读（见 `catchUp`）。
onLive(event => {
  if (event.type === 'resumed') cache.clear();
  else cache.delete(event.sessionId);
});

const messageOf = (problem: unknown) => (problem instanceof ApiError && problem.message) || t('读取消息失败');

/** 目标附近的消息：某一天开始的一页，或者搜索结果前后的消息。 */
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
  // 那一天之后没有消息：显示最新的消息，把最后一条放在视野中
  const latest = await getMessages(sessionId, null, signal);
  const messages = latest.slice().reverse();
  return { ...IDLE, status: 'ready', messages, hasMore: latest.length === PAGE_SIZE, focus: messages.length ? { cursor: messages[messages.length - 1]._cursor, align: 'center', seq } : null };
}

/**
 * 一个会话的消息：先读最新的一页，需要时再读更早的页；跳转（某一天、某个搜索结果）后读那个位置的一页，需要时在上方
 * 读更早的、在下方读更新的。状态记着它属于哪个会话，因此刚切换会话时，新会话不会显示前一个会话的消息，
 * 也不会按它们滚动。
 */
export function useMessages(sessionId: string | null, jump: Jump | null = null): {
  thread: Thread; loadOlder(): void; loadNewer(): void; reload(): void;
} {
  const [owned, setOwned] = useState<{ id: string | null; thread: Thread }>(() => ({ id: sessionId, thread: (sessionId && cache.get(sessionId)) || (sessionId ? LOADING : IDLE) }));
  /** 最后执行过的跳转的 `seq`：回到它的会话时不再跳一次。 */
  const handled = useRef<number | null>(null);
  const pendingJump = jump && jump.sessionId === sessionId && jump.seq !== handled.current;
  const thread = owned.id === sessionId ? owned.thread : pendingJump ? LOADING : (sessionId && cache.get(sessionId)) || (sessionId ? LOADING : IDLE);
  const latest = useRef(thread);
  latest.current = thread;
  const current = useRef(sessionId);
  const paging = useRef<AbortController | null>(null);
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
    paging.current?.abort();
    if (!sessionId) { update(null, IDLE); return; }
    const target = jump && jump.sessionId === sessionId && jump.seq !== handled.current ? jump : null;
    const controller = new AbortController();
    const scope = currentScope();
    if (target) {
      handled.current = target.seq;
      update(sessionId, LOADING);
      threadAt(sessionId, target.target, target.seq, controller.signal).then(next => {
        remember(sessionId, next, scope);
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
      remember(sessionId, next, scope);
      update(sessionId, next);
    }, problem => {
      if (!controller.signal.aborted) update(sessionId, { ...IDLE, status: 'error', error: messageOf(problem) });
    });
    return () => controller.abort();
  }, [sessionId, round, jump, update]);

  /** 一侧的下一页：第一条消息上方更早的，或者最后一条下方更新的。 */
  const page = useCallback((side: 'older' | 'newer') => {
    const id = current.current;
    const now = latest.current;
    const busy = now.loadingOlder || now.loadingNewer;
    const left = side === 'older' ? now.hasMore : now.hasNewer;
    if (!id || now.status !== 'ready' || !left || busy || !now.messages.length) return;
    const controller = new AbortController();
    const scope = currentScope();
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
        remember(id, next, scope);
        return next;
      });
    }, problem => {
      if (!controller.signal.aborted && current.current === id) update(id, old => ({ ...old, [flag]: false, error: messageOf(problem) }));
    });
  }, [update]);

  const loadOlder = useCallback(() => page('older'), [page]);
  const loadNewer = useCallback(() => page('newer'), [page]);

  /** 不用缓存，重新读最新的一页（出错后，或者从跳转的位置回到最新消息）。 */
  const reload = useCallback(() => {
    fresh.current = true;
    setRound(n => n + 1);
  }, []);

  /**
   * 实时消息（#201）：读出最后显示的消息之后的全部消息，加在下面；`revised`（撤回，或者中断恢复后）时还重读最新的
   * 一页，换上其中改动过的消息。跳转后停在过去的会话不动，由「回到最新消息」读取最新的。同时只进行一次补读：
   * 补读期间来的事件在它完成后一起处理，从它读到的最后一条消息接着读，因此一阵消息最多读两次，每次都不用等待。
   */
  const catching = useRef<{ id: string; again: boolean; revised: boolean } | null>(null);
  const catchUp = useCallback((revised: boolean) => {
    const id = current.current;
    const now = latest.current;
    if (!id || now.status !== 'ready' || now.hasNewer || !now.messages.length) return;
    const running = catching.current;
    if (running?.id === id) {
      running.again = true;
      running.revised ||= revised;
      return;
    }
    const run = { id, again: false, revised };
    catching.current = run;
    const scope = currentScope();
    (async () => {
      let from = now.messages[now.messages.length - 1]._cursor;
      for (;;) {
        const wanted = run.revised;
        run.again = false;
        run.revised = false;
        const added: ParsedMessage[] = [];
        let last = from;
        for (;;) {
          const page = (await getMessages(id, { after: last })).slice().reverse();
          added.push(...page);
          if (page.length < PAGE_SIZE) break;
          last = page[page.length - 1]._cursor;
        }
        const newest = wanted ? await getMessages(id, null) : [];
        if (current.current !== id || scope !== currentScope()) return;
        const after = from;
        update(id, old => {
          // 这期间重读过或跳到了过去：读到的消息可能接不上显示的消息
          if (old.status !== 'ready' || old.hasNewer || !old.messages.some(m => m._cursor === after)) return old;
          const changed = new Map(newest.map(m => [m._cursor, m]));
          const messages = old.messages.map(m => changed.get(m._cursor) ?? m);
          const seen = new Set(messages.map(m => m._cursor));
          const next: Thread = { ...old, messages: [...messages, ...added.filter(m => !seen.has(m._cursor))] };
          remember(id, next, scope);
          return next;
        });
        if (!run.again) return;
        from = added.at(-1)?._cursor ?? from;
      }
    })().catch(() => { /* 下一个事件或下一次中断恢复时再试 */ }).finally(() => {
      if (catching.current === run) catching.current = null;
    });
  }, [update]);

  useLive(event => {
    if (event.type === 'resumed') catchUp(true);
    else if (event.sessionId === current.current) catchUp(event.type === 'message.revoke');
  });

  return { thread, loadOlder, loadNewer, reload };
}
