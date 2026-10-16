import { useCallback, useEffect, useState } from 'react';
import { getSessions, getStatuses, type ChatSession, type SessionStatus } from '../api/chat';
import { ApiError } from '../api/client';

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
 * the folded and muted states, which only change how it is shown and do not hold it up.
 */
export function useSessions(enabled: boolean): [SessionsState, () => void] {
  const [state, setState] = useState<SessionsState>({ status: 'idle' });
  const [round, setRound] = useState(0);
  const reload = useCallback(() => setRound(n => n + 1), []);

  useEffect(() => {
    if (!enabled) { setState({ status: 'idle' }); return; }
    const controller = new AbortController();
    setState(old => (old.status === 'ready' ? old : { status: 'loading' }));
    (async () => {
      let sessions: ChatSession[];
      try {
        sessions = await getSessions(controller.signal);
      } catch (problem) {
        if (!controller.signal.aborted) setState({ status: 'error', message: messageOf(problem) });
        return;
      }
      if (controller.signal.aborted) return;
      setState(old => ({ status: 'ready', sessions, statuses: old.status === 'ready' ? old.statuses : {} }));
      try {
        const statuses = await getStatuses(sessions.map(s => s.username), controller.signal);
        if (!controller.signal.aborted) setState({ status: 'ready', sessions, statuses });
      } catch {
        // Without the states the list is still right, only nothing is folded or marked muted.
      }
    })();
    return () => controller.abort();
  }, [enabled, round]);

  return [state, reload];
}
