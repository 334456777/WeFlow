// One read of `weflow serve` that a view shows as it comes: loading, the answer, or why it failed.
import { useEffect, useRef, useState } from 'react';
import { ApiError } from './api/client';
import { t } from './i18n';

export type Read<T> = { status: 'loading' } | { status: 'ready'; value: T } | { status: 'error'; message: string };

/** What `read` answers, read again when `key` changes or `retry` is called. */
export function useRead<T>(key: string, read: (signal: AbortSignal) => Promise<T>): [Read<T>, () => void] {
  const [state, setState] = useState<Read<T>>({ status: 'loading' });
  const [round, setRound] = useState(0);
  const latest = useRef(read);
  latest.current = read;
  useEffect(() => {
    const controller = new AbortController();
    setState({ status: 'loading' });
    // an answer for a key since left, as of a read shared with others that is not cancelled, is not shown
    latest.current(controller.signal).then(value => { if (!controller.signal.aborted) setState({ status: 'ready', value }); }, problem => {
      if (!controller.signal.aborted) setState({ status: 'error', message: (problem instanceof ApiError && problem.message) || t('读取失败') });
    });
    return () => controller.abort();
  }, [key, round]);
  return [state, () => setRound(r => r + 1)];
}
