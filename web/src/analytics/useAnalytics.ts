// 聊天分析页的读取（#207）：私聊的统计在打开页面时读取，换了服务端或账号、或者不统计的好友变了之后重读；
// 以及供选择的群聊列表。
import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiError } from '../api/client';
import { t } from '../i18n';
import { getOverall, getRankings, getTimeSpread, type Overall, type Ranked, type TimeSpread } from '../api/analytics';
import { getGroups, type Group } from '../api/groups';

/** 排行列出的私聊个数。 */
export const RANKED = 20;

/** 私聊统计的读取状态：还没读、正在读、已读到，或者失败的原因。 */
export type AnalyticsState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; overall: Overall; rankings: Ranked[]; spread: TimeSpread }
  | { status: 'error'; message: string };

/**
 * 私聊的统计：`enabled` 时读取，`scope` 变化后重读。每次读取都重新统计私聊，
 * 所以 `reload()` 能算上这期间收到的消息。
 */
export function useAnalytics(enabled: boolean, scope: number): [AnalyticsState, () => void] {
  const [state, setState] = useState<AnalyticsState>({ status: 'idle' });
  const [round, setRound] = useState(0);
  const reload = useCallback(() => setRound(n => n + 1), []);
  const read = useRef<string | null>(null);
  useEffect(() => {
    const key = scope + ':' + round;
    if (!enabled || read.current === key) return;
    read.current = key;
    const controller = new AbortController();
    let done = false;
    setState({ status: 'loading' });
    Promise.all([getOverall(controller.signal), getRankings(RANKED, controller.signal), getTimeSpread(controller.signal)]).then(([overall, rankings, spread]) => {
      done = true;
      setState({ status: 'ready', overall, rankings, spread });
    }, problem => {
      if (controller.signal.aborted) return;
      done = true;
      read.current = null;
      setState({ status: 'error', message: problem instanceof ApiError && problem.message ? problem.message : t('读取聊天分析失败') });
    });
    return () => { if (done) return; controller.abort(); if (read.current === key) read.current = null; };
  }, [enabled, scope, round]);
  return [state, reload];
}

/** 群聊列表的读取状态：还没读、正在读、已读到，或者失败的原因。 */
export type GroupsState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; groups: Group[] }
  | { status: 'error'; message: string };

/** 群聊列表：打开群聊模式（`enabled`）时读取，换了服务端或账号（`scope`）后重读。 */
export function useGroups(enabled: boolean, scope: number): [GroupsState, () => void] {
  const [state, setState] = useState<GroupsState>({ status: 'idle' });
  const [round, setRound] = useState(0);
  const reload = useCallback(() => setRound(n => n + 1), []);
  const read = useRef<string | null>(null);
  useEffect(() => {
    const key = scope + ':' + round;
    if (!enabled || read.current === key) return;
    read.current = key;
    const controller = new AbortController();
    let done = false;
    setState({ status: 'loading' });
    getGroups(controller.signal).then(groups => { done = true; setState({ status: 'ready', groups }); }, problem => {
      if (controller.signal.aborted) return;
      done = true;
      read.current = null;
      setState({ status: 'error', message: problem instanceof ApiError && problem.message ? problem.message : t('读取群聊失败') });
    });
    return () => { if (done) return; controller.abort(); if (read.current === key) read.current = null; };
  }, [enabled, scope, round]);
  return [state, reload];
}
