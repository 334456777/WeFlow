// 视图随进展显示的一次 `weflow serve` 读取：读取中、读到的回答，或者失败的原因。
import { useEffect, useRef, useState } from 'react';
import { ApiError } from './api/client';
import { t } from './i18n';

/** 一次读取的状态：读取中、读到的回答，或者用文字写的失败原因。 */
export type Read<T> = { status: 'loading' } | { status: 'ready'; value: T } | { status: 'error'; message: string };

/** `read` 读取的状态和重读的函数；`key` 变化或调用重读的函数时重新读取。 */
export function useRead<T>(key: string, read: (signal: AbortSignal) => Promise<T>): [Read<T>, () => void] {
  const [state, setState] = useState<Read<T>>({ status: 'loading' });
  const [round, setRound] = useState(0);
  const latest = useRef(read);
  latest.current = read;
  useEffect(() => {
    const controller = new AbortController();
    setState({ status: 'loading' });
    // 已经换掉的 key 的回答不显示：与别处共用的读取不会随中止而取消，它的回答仍会到来
    latest.current(controller.signal).then(value => { if (!controller.signal.aborted) setState({ status: 'ready', value }); }, problem => {
      if (!controller.signal.aborted) setState({ status: 'error', message: (problem instanceof ApiError && problem.message) || t('读取失败') });
    });
    return () => controller.abort();
  }, [key, round]);
  return [state, () => setRound(r => r + 1)];
}
