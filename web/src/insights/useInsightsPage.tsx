// The insights page (#210): the records of the AI insights, of everyone or one conversation, found by words; made
// and deleted from the console.
import { useEffect, useState } from 'react';
import { markInsightRead } from '../api/insights';
import type { ChatWith, Page, Shell } from '../shell/page';
import { BrowserPanel, ConnectedList } from '../shell/panels';
import { InsightClear, InsightContacts, InsightList, InsightsSummary, InsightTrigger } from './InsightViews';
import { useInsights, type InsightFilter } from './useInsights';

export function useInsightsPage(shell: Shell, active: boolean, chatWith: ChatWith): Page {
  const { backend, connected, scope, setMobileDetail } = shell;
  const [filter, setFilter] = useState<InsightFilter>({ keyword: '', session: null });
  useEffect(() => { setFilter({ keyword: '', session: null }); }, [scope]);
  const insights = useInsights(active && connected, scope, filter);
  const page = insights.state.status === 'ready' ? insights.state.page : null;
  const name = filter.session ? page?.contacts.find(c => c.sessionId === filter.session)?.displayName ?? null : null;
  const count = page ? (filter.session ? page.contacts.find(c => c.sessionId === filter.session)?.count ?? 0 : page.contacts.reduce((n, c) => n + c.count, 0)) : 0;
  const read = (id: string) => { insights.markRead(id); markInsightRead(id).catch(() => {}); };
  const cards = [
    {
      id: 'insights-summary', title: '见解概况', summary: page ? page.unreadCount + ' 条未读' : '',
      content: <InsightsSummary state={insights.state} engine={'health' in backend ? backend.health.insight : undefined} aiReady={shell.profile?.aiKey} />,
    },
    { id: 'insights-trigger', title: '生成见解', summary: name ?? '', content: <InsightTrigger session={filter.session} name={name} onDone={insights.reload} /> },
    { id: 'insights-clear', title: '删除见解', content: <InsightClear session={filter.session} name={name} count={count} onDone={insights.reload} /> },
  ];
  const view = <BrowserPanel label="灵感信箱" detail={shell.mobileDetail} onBack={() => setMobileDetail(false)} back="返回联系人" title="见解" paneLabel="见解"
    list={<ConnectedList backend={backend} offline="连接 weflow serve 后，可以在这里查看 AI 见解。">
      <InsightContacts state={insights.state} selected={filter.session} onRetry={insights.reload}
        keyword={filter.keyword} onKeyword={keyword => setFilter(f => ({ ...f, keyword }))}
        onSelect={session => { setFilter(f => ({ ...f, session })); setMobileDetail(true); }} />
    </ConnectedList>}>
    {connected && <InsightList state={insights.state} narrowed={Boolean(filter.keyword || filter.session)} onRead={read} onMore={insights.more}
      canChat={session => shell.sessionList.some(s => s.username === session)} onChat={chatWith} />}
  </BrowserPanel>;
  return { view, cards, shown: connected ? cards.map(card => card.id) : [] };
}
