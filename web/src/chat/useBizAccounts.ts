// The official accounts followed (#203), for the 「公众号」 filter: read once it is first chosen, again for another
// server or account (scope.ts). As conversations, so the list and the message pane take them as they are.
import { useEffect, useState } from 'react';
import { getBizAccounts } from '../api/biz';
import type { ChatSession } from '../api/chat';

export function useBizAccounts(enabled: boolean, scope: number): ChatSession[] {
  const [accounts, setAccounts] = useState<{ scope: number; sessions: ChatSession[] } | null>(null);
  const known = accounts?.scope === scope;

  useEffect(() => {
    if (!enabled || known) return;
    const controller = new AbortController();
    getBizAccounts(controller.signal).then(found => {
      const sessions = found
        .map((a): ChatSession => ({
          username: a.username, displayName: a.name, summary: '', unreadCount: 0,
          sortTimestamp: a.lastTime, lastTimestamp: a.lastTime, lastMsgType: 0, avatarUrl: a.avatarUrl,
        }))
        .sort((a, b) => b.sortTimestamp - a.sortTimestamp);
      setAccounts({ scope, sessions });
    }, () => { /* the list still has the accounts it knows */ });
    return () => controller.abort();
  }, [enabled, known, scope]);

  return known ? accounts.sessions : [];
}
