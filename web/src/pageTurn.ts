// The page turns (#832): each time another page comes in, or another item's detail in a page (motion.ts pageIn), the
// count of turns goes up by one, for the skin's decoration (Backdrop.tsx) to change with it; the count of pages only
// when another page comes in, for what the skin sticks on the page itself, which stays on while a detail changes
// under it. Both stay as they are with reduced motion, where pageIn plays nothing.
import { useSyncExternalStore } from 'react';

export interface PageTurns {
  /** Turns so far: pages and details. */
  turn: number;
  /** Pages so far: turns that brought in another page, not another item's detail in the same page. */
  page: number;
}

let turns: PageTurns = { turn: 0, page: 0 };
const listeners = new Set<() => void>();

/** One turn more: another page, or with `detail` another item's detail in the same page. */
export function turnPage({ detail = false }: { detail?: boolean } = {}): void {
  turns = { turn: turns.turn + 1, page: turns.page + (detail ? 0 : 1) };
  for (const listener of listeners) listener();
}

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
};
const current = () => turns;

/** The turns so far. */
export const usePageTurn = (): PageTurns => useSyncExternalStore(subscribe, current, current);
