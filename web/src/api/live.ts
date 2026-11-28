// 实时消息（#201）：把 `/api/push/messages` 推来的事件交给显示它们的地方。
// 「实时消息」打开时由 `Connection`（backend.ts）保持这个事件流；会话列表和打开的会话收到事件后重新读取。
import { useCallback, useEffect, useRef } from 'react';

/** 实时事件流推来的事件。 */
export type LiveEvent =
  /** 某个会话里来了一条消息，或者有消息被撤回。 */
  | { type: 'message.new' | 'message.revoke'; sessionId: string }
  /** 事件流中断后恢复了：中断期间可能来过任何消息。 */
  | { type: 'resumed' };

const listeners = new Set<(event: LiveEvent) => void>();

/** 把 `event` 交给每个监听者。 */
export function emitLive(event: LiveEvent): void {
  for (const listener of [...listeners]) listener(event);
}

/** 每个实时事件都调用 `listener`；用于不随组件出现和消失的模块级状态。返回停止监听的函数。 */
export function onLive(listener: (event: LiveEvent) => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** 组件存在期间，每个实时事件都调用最新的 `listener`。 */
export function useLive(listener: (event: LiveEvent) => void): void {
  const latest = useRef(listener);
  latest.current = listener;
  useEffect(() => onLive(event => latest.current(event)), []);
}

/** 一阵实时消息在触发要启动命令的读取（`chat resources` 等）之前汇集的时长。 */
export const LIVE_COMMAND_MS = 2000;

/** 一阵实时消息在触发服务进程内的读取（`/api/chat/sessions` 等）之前汇集的时长。 */
export const LIVE_READ_MS = 500;

/**
 * 返回一个函数：第一次调用后过 `ms` 运行 `action`，期间的调用并入这一次运行。消息不断的群因此每 `ms` 最多读一次，
 * 也不会因为消息一直在来而一直推迟。
 */
export function useGathered(action: () => void, ms = LIVE_COMMAND_MS): () => void {
  const latest = useRef(action);
  latest.current = action;
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  return useCallback(() => {
    if (timer.current !== undefined) return;
    timer.current = setTimeout(() => { timer.current = undefined; latest.current(); }, ms);
  }, [ms]);
}
