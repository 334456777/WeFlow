// The private chats' analytics (#207), read once the page is opened and again for another server or account, or
// once the people left out change.
import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiError } from '../api/client';
import { getOverall, getRankings, getTimeSpread, type Overall, type Ranked, type TimeSpread } from '../api/analytics';

export const RANKED = 20;

export type AnalyticsState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; overall: Overall; rankings: Ranked[]; spread: TimeSpread }
  | { status: 'error'; message: string };

/** With `reload(true)`, the next read counts every chat again rather than using the aggregate kept from last time. */
export function useAnalytics(enabled: boolean, scope: number): [AnalyticsState, (force?: boolean) => void] {
  const [state, setState] = useState<AnalyticsState>({ status: 'idle' });
  const [round, setRound] = useState(0);
  const force = useRef(false);
  const reload = useCallback((again = false) => { force.current = again; setRound(n => n + 1); }, []);
  const read = useRef<string | null>(null);
  useEffect(() => {
    const key = scope + ':' + round;
    if (!enabled || read.current === key) return;
    read.current = key;
    const controller = new AbortController();
    let done = false;
    setState({ status: 'loading' });
    const again = force.current;
    force.current = false;
    // each read is a process of its own: the first counts the chats and keeps the aggregate, the others read it
    getOverall(again, controller.signal).then(overall => Promise.all([overall, getRankings(RANKED, controller.signal), getTimeSpread(controller.signal)])).then(([overall, rankings, spread]) => {
      done = true;
      setState({ status: 'ready', overall, rankings, spread });
    }, problem => {
      if (controller.signal.aborted) return;
      done = true;
      read.current = null;
      setState({ status: 'error', message: problem instanceof ApiError && problem.message ? problem.message : '读取聊天分析失败' });
    });
    return () => { if (done) return; controller.abort(); if (read.current === key) read.current = null; };
  }, [enabled, scope, round]);
  return [state, reload];
}
