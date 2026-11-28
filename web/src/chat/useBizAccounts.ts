// 关注的公众号（#203），用于「公众号」筛选：第一次选中这个筛选时读取，换了服务端或账号（scope.ts）后重新读取。
// 读到的公众号写成会话的形式，会话列表和消息区可以直接使用。
import { useEffect, useState } from 'react';
import { getBizAccounts } from '../api/biz';
import type { ChatSession } from '../api/chat';

/** 关注的公众号，写成会话的形式，按 `lastTime` 从新到旧；`enabled` 之前为空，`scope` 变了重新读取。 */
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
    }, () => { /* 读取失败时，列表里仍有带会话的公众号 */ });
    return () => controller.abort();
  }, [enabled, known, scope]);

  return known ? accounts.sessions : [];
}
