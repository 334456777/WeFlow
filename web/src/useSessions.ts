import { useCallback, useEffect, useRef, useState } from 'react';
import { getSessions, getStatuses, type ChatSession, type SessionStatus } from './api/chat';
import { ApiError } from './api/client';
import { LIVE_READ_MS, useGathered, useLive } from './api/live';
import { t } from './i18n';

/** 会话列表的读取状态：尚未读取、正在读取、读到的列表和每个会话的状态，或者失败的原因。 */
export type SessionsState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; sessions: ChatSession[]; statuses: Record<string, SessionStatus> }
  | { status: 'error'; message: string };

function messageOf(problem: unknown): string {
  if (problem instanceof ApiError) {
    if (problem.code === 'config_error') return t('weflow 还没有配置账号：{message}', { message: problem.message });
    return problem.message || t('读取会话失败');
  }
  return t('读取会话失败');
}

/**
 * `enabled`（服务端在线且版本兼容）时读取 `weflow serve` 的会话列表：先读列表，再读折叠和免打扰的状态，
 * 这些状态只影响显示方式，不耽误列表的显示。换了 `scope`（另一个服务端或账号，见 `scope.ts`）时重新读取，
 * 读到之前不显示上一个作用域的列表。实时消息（#201）到来时安静地重读：只为新出现的会话读取状态，
 * 读取失败时保留显示中的列表。
 */
export function useSessions(enabled: boolean, scope = 0): [SessionsState, () => void] {
  const [state, setState] = useState<SessionsState>({ status: 'idle' });
  // 显示中的列表是在哪个作用域里读到的。新作用域的列表读到之前什么都不显示，
  // 否则上一个账号的会话会在新的账号里被打开和读取（#402）。
  const [stateScope, setStateScope] = useState(scope);
  const [round, setRound] = useState(0);
  const reload = useCallback(() => setRound(n => n + 1), []);
  const shownScope = useRef(scope);
  const quiet = useRef(false);
  const latest = useRef(state);
  latest.current = state;

  // 一阵密集的消息只重读一次列表，消息持续到来时每隔 LIVE_READ_MS 重读一次
  const refresh = useGathered(() => { quiet.current = true; reload(); }, LIVE_READ_MS);
  useLive(refresh);

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
        // 没有状态时列表仍然正确，只是不折叠会话，也不标出免打扰。
      }
    })();
    return () => controller.abort();
  }, [enabled, round, scope]);

  return [enabled && stateScope !== scope ? LOADING : state, reload];
}

const LOADING: SessionsState = { status: 'loading' };
