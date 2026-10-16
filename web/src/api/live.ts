// Live messages (#201): what `/api/push/messages` says, handed to whoever shows it. `Connection` (backend.ts) holds
// the stream while 「实时消息」 is on; the conversation list and the open conversation read again from it.
import { useCallback, useEffect, useRef } from 'react';

export type LiveEvent =
  /** A message came, or one was recalled, in a conversation. */
  | { type: 'message.new' | 'message.revoke'; sessionId: string }
  /** The stream is back after a break: anything may have come meanwhile. */
  | { type: 'resumed' };

const listeners = new Set<(event: LiveEvent) => void>();

export function emitLive(event: LiveEvent): void {
  for (const listener of [...listeners]) listener(event);
}

/** Calls `listener` for each live event; for module-level state that does not come and go with a component. */
export function onLive(listener: (event: LiveEvent) => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** Calls the latest `listener` for each live event while the component is there. */
export function useLive(listener: (event: LiveEvent) => void): void {
  const latest = useRef(listener);
  latest.current = listener;
  useEffect(() => onLive(event => latest.current(event)), []);
}

/** How long a burst of live messages is gathered before a read that starts a command (`chat resources`, …). */
export const LIVE_COMMAND_MS = 2000;

/**
 * A function that runs `action` `ms` after it is first called; the calls meanwhile join that run. A busy group
 * then reads at most once every `ms`, and is not put off for as long as messages keep coming.
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
