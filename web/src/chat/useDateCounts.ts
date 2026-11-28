// 会话里有消息的日子，用于日历（#199）：第一次打开日历时读取，之后保留。
import { useCallback, useEffect, useState } from 'react';
import { getDateCounts } from '../api/messages';
import { onLive } from '../api/live';
import { currentScope, onNewScope } from '../api/scope';

const cache = new Map<string, Record<string, number>>();
onNewScope(() => cache.clear());
// 新消息要算进它那一天（#201）：清掉这个会话的缓存，中断恢复后清掉全部
onLive(event => {
  if (event.type === 'resumed') cache.clear();
  else cache.delete(event.sessionId);
});

/**
 * 请求过（`request`）之后 `sessionId` 每天的消息数，否则为 null；读到之前，日历显示没有标记的日子。每次请求都先用
 * 缓存，会话来过消息后重新读取，这期间显示之前读到的日子。
 */
export function useDateCounts(sessionId: string | null): { counts: Record<string, number> | null; request(): void } {
  const [wanted, setWanted] = useState<{ id: string; seq: number } | null>(null);
  const [counts, setCounts] = useState<{ id: string; counts: Record<string, number> } | null>(null);

  useEffect(() => {
    if (!sessionId || wanted?.id !== sessionId) return;
    const kept = cache.get(sessionId);
    if (kept) { setCounts(old => (old?.id === sessionId && old.counts === kept ? old : { id: sessionId, counts: kept })); return; }
    const controller = new AbortController();
    const scope = currentScope();
    getDateCounts(sessionId, controller.signal).then(found => {
      if (scope !== currentScope()) return;
      cache.set(sessionId, found);
      setCounts({ id: sessionId, counts: found });
    }, () => { /* 没有标记时日历照样能用 */ });
    return () => controller.abort();
  }, [sessionId, wanted]);

  const request = useCallback(() => {
    if (sessionId) setWanted(old => ({ id: sessionId, seq: (old?.seq ?? 0) + 1 }));
  }, [sessionId]);
  const known = sessionId ? (counts?.id === sessionId ? counts.counts : cache.get(sessionId) ?? null) : null;
  return { counts: known, request };
}
