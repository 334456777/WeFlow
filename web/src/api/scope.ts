// 网页保存的记录属于谁（#198）。网页在视图之间保存的东西（消息、群成员、有消息的日期）属于一个服务端和一个账号；
// 两者之一可能变了时开始新的作用域：保存的东西全部丢弃，旧作用域里发出的请求的回答也不保存。
//
// 以下情况开始新的作用域：服务端的 `data`（在 `/health/events` 的 `hello` 和每个 `heartbeat` 里，每次启动
// `weflow serve` 和配置变化时都会变，所以账号变了它也会变）与之前看到的不同；服务端断开后又回来；示例数据（#879）
// 取代了服务端的记录。
import { useEffect, useRef, useState } from 'react';
import type { Backend } from './backend';
import { recordsOf } from './records';

let scope = 0;
const forgetters = new Set<() => void>();

/** 当前的作用域：保存一个请求的回答之前，先和请求开始时的作用域比较。 */
export const currentScope = () => scope;

/** 登记开始新的作用域时要丢弃的东西。 */
export function onNewScope(forget: () => void): void {
  forgetters.add(forget);
}

/** 开始新的作用域，丢弃旧作用域保存的东西。 */
export function newScope(): void {
  scope += 1;
  for (const forget of forgetters) forget();
}

/** `useBackend` 检查到的服务端状态对应的作用域：数据可能变了时换成新的作用域。 */
export function useScope(backend: Backend): number {
  const seen = useRef<{ data: string | null; lost: boolean; sample: boolean } | null>(null);
  const [current, setCurrent] = useState(currentScope);
  useEffect(() => {
    if (backend.state === 'checking') return;
    const last = seen.current;
    const start = () => { newScope(); setCurrent(currentScope()); };
    if (backend.state !== 'ready') {
      const sample = recordsOf(backend) === 'sample';
      seen.current = { data: last?.data ?? null, lost: true, sample };
      // 服务端的记录换成示例数据：丢弃从服务端读到的东西
      if (last && !last.sample && sample) start();
      return;
    }
    const data = backend.health.data ?? null;
    seen.current = { data, lost: false, sample: false };
    // 示例数据之后连上的服务端，按断开后又回来的服务端处理
    if (last && (last.lost || last.data !== data)) start();
  }, [backend]);
  return current;
}
