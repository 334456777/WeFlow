import { useCallback, useEffect, useRef, useState } from 'react';
import { getSessions, getStatuses, type ChatSession, type SessionStatus } from '../api/chat';
import { ApiError } from '../api/client';
import { useLive } from '../api/live';

export type SessionsState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; sessions: ChatSession[]; statuses: Record<string, SessionStatus> }
  | { status: 'error'; message: string };

function messageOf(problem: unknown): string {
  if (problem instanceof ApiError) {
    if (problem.code === 'config_error') return 'weflow 还没有配置账号：' + problem.message;
    return problem.message || '读取会话失败';
  }
  return '读取会话失败';
}

/**
 * The conversation list of `weflow serve` while `enabled` (the server is there and compatible): the list first, then
 * the folded and muted states, which only change how it is shown and do not hold it up. A new `scope` (another
 * server or account, see `scope.ts`) reads it again without showing the one before meanwhile. Live messages (#201)
 * read it again quietly: the states only for conversations new to it, and a failure keeps the list shown.
 */
export function useSessions(enabled: boolean, scope = 0): [SessionsState, () => void] {
  const [state, setState] = useState<SessionsState>({ status: 'idle' });
  // The scope the list shown was read in. Until the list of a new scope is read, none is shown: a conversation of the
  // account before would otherwise be opened, and read, in the new one (#402).
  const [stateScope, setStateScope] = useState(scope);
  const [round, setRound] = useState(0);
  const reload = useCallback(() => setRound(n => n + 1), []);
  const shownScope = useRef(scope);
  const quiet = useRef(false);
  const latest = useRef(state);
  latest.current = state;

  // a burst of messages reads the list once
  const pending = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useLive(() => {
    clearTimeout(pending.current);
    pending.current = setTimeout(() => { quiet.current = true; reload(); }, LIVE_DELAY_MS);
  });
  useEffect(() => () => clearTimeout(pending.current), []);

  useEffect(() => {
    setStateScope(scope);
    if (!enabled) { setState({ status: 'idle' }); return; }
    const controller = new AbortController();
    const fresh = shownScope.current !== scope;
    shownScope.current = scope;
    const before = latest.current;
    const live = quiet.current && !fresh && before.status === 'ready';
    quiet.current = false;
    setState(old => (old.status === 'ready' && !fresh ? old : { status: 'loading' }));
    (async () => {
      let sessions: ChatSession[];
      try {
        sessions = await getSessions(controller.signal);
      } catch (problem) {
        if (!controller.signal.aborted && !live) setState({ status: 'error', message: messageOf(problem) });
        return;
      }
      if (controller.signal.aborted) return;
      setState(old => ({ status: 'ready', sessions, statuses: old.status === 'ready' ? old.statuses : {} }));
      const known = live && before.status === 'ready' ? before.statuses : {};
      const asked = sessions.map(s => s.username).filter(name => !(name in known));
      if (!asked.length) return;
      try {
        const statuses = await getStatuses(asked, controller.signal);
        if (!controller.signal.aborted) setState({ status: 'ready', sessions, statuses: { ...known, ...statuses } });
      } catch {
        // Without the states the list is still right, only nothing is folded or marked muted.
      }
    })();
    return () => controller.abort();
  }, [enabled, round, scope]);

  return [enabled && stateScope !== scope ? LOADING : state, reload];
}

const LOADING: SessionsState = { status: 'loading' };

/** How long live messages wait for more before the list is read again. */
const LIVE_DELAY_MS = 500;
