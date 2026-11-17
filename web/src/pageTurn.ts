// The page turns (#832): each time another page comes in, or another item's detail in a page (motion.ts pageIn), the
// count goes up by one, for the skin's decoration (Backdrop.tsx) to change with it. It stays as it is with reduced
// motion, where pageIn plays nothing.
import { useSyncExternalStore } from 'react';

let turns = 0;
const listeners = new Set<() => void>();

/** One turn more. */
export function turnPage(): void {
  turns += 1;
  for (const listener of listeners) listener();
}

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
};
const current = () => turns;

/** The number of turns so far. */
export const usePageTurn = (): number => useSyncExternalStore(subscribe, current, current);
