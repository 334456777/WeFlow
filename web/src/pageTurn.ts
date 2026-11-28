// 换页的计数（#832）：每次换进另一个页面，或者在页面里换成另一项的详情（motion.ts 的 pageIn），换页次数加一，
// 皮肤的装饰层（Backdrop.tsx）随之轮换；页数只在换进另一个页面时加一，供皮肤贴在页面上的东西使用，
// 这些东西在详情变化时留在原处。减少动画时 pageIn 不播放，两个计数都不变。
import { useSyncExternalStore } from 'react';

/** 换页的两个计数。 */
export interface PageTurns {
  /** 到目前为止的换页次数，换页面和换详情都算。 */
  turn: number;
  /** 到目前为止的页数：只算换进另一个页面的换页，不算同一页面里换成另一项的详情。 */
  page: number;
}

let turns: PageTurns = { turn: 0, page: 0 };
const listeners = new Set<() => void>();

/** 记一次换页：换进另一个页面；带 `detail` 时是同一页面里换成另一项的详情。 */
export function turnPage({ detail = false }: { detail?: boolean } = {}): void {
  turns = { turn: turns.turn + 1, page: turns.page + (detail ? 0 : 1) };
  for (const listener of listeners) listener();
}

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
};
const current = () => turns;

/** 到目前为止的两个换页计数。 */
export const usePageTurn = (): PageTurns => useSyncExternalStore(subscribe, current, current);
