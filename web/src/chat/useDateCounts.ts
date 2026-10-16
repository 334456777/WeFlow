// The days of a conversation that have messages, for the calendars (#199): read the first time one opens, then kept.
import { useCallback, useEffect, useState } from 'react';
import { getDateCounts } from '../api/messages';
import { currentScope, onNewScope } from '../api/scope';

const cache = new Map<string, Record<string, number>>();
onNewScope(() => cache.clear());

/** The counts of `sessionId` once asked for (`request`), else null; a calendar shows plain days until they come. */
export function useDateCounts(sessionId: string | null): { counts: Record<string, number> | null; request(): void } {
  const [wanted, setWanted] = useState<string | null>(null);
  const [counts, setCounts] = useState<{ id: string; counts: Record<string, number> } | null>(null);

  useEffect(() => {
    if (!sessionId || wanted !== sessionId) return;
    const kept = cache.get(sessionId);
    if (kept) { setCounts({ id: sessionId, counts: kept }); return; }
    const controller = new AbortController();
    const scope = currentScope();
    getDateCounts(sessionId, controller.signal).then(found => {
      if (scope !== currentScope()) return;
      cache.set(sessionId, found);
      setCounts({ id: sessionId, counts: found });
    }, () => { /* without the marks the calendar still works */ });
    return () => controller.abort();
  }, [sessionId, wanted]);

  const request = useCallback(() => setWanted(sessionId), [sessionId]);
  const known = sessionId ? (counts?.id === sessionId ? counts.counts : cache.get(sessionId) ?? null) : null;
  return { counts: known, request };
}
