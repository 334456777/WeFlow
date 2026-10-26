// The chat analytics page (#207): the private chats counted once the page is opened (again when the people left out
// change), or a group picked (the first until then) over a range of days, and one of its members.
import { useEffect, useRef, useState } from 'react';
import { ChartBar } from '@phosphor-icons/react';
import type { DayRange } from '../api/groups';
import { DateRange } from '../calendar/DateRange';
import { Segmented } from '../Segmented';
import type { ChatWith, Page, Shell } from '../shell/page';
import { Connected, ScrollPage } from '../shell/panels';
import { ExcludedPeople, PrivateAnalytics } from './AnalyticsViews';
import { GroupExport, GroupPicker, GroupStats, MemberView } from './GroupAnalytics';
import { useAnalytics, useGroups } from './useAnalytics';

type Member = { username: string; displayName: string; avatarUrl?: string };

export function useAnalyticsPage(shell: Shell, active: boolean, chatWith: ChatWith): Page {
  const { connected, scope } = shell;
  const [mode, setMode] = useState<'private' | 'group'>('private');
  const [analytics, reloadAnalytics] = useAnalytics(active && mode === 'private' && connected, scope);
  const [groups, reloadGroups] = useGroups(active && mode === 'group' && connected, scope);
  const [groupId, setGroupId] = useState<string | null>(null);
  const [member, setMember] = useState<Member | null>(null);
  const [range, setRange] = useState<DayRange>({ from: '', to: '' });
  useEffect(() => { setGroupId(null); setMember(null); }, [scope]);
  const groupList = groups.status === 'ready' ? groups.groups : [];
  const group = groupList.find(g => g.username === groupId) ?? groupList[0] ?? null;
  const scroller = useRef<HTMLElement>(null);
  const showMember = (next: Member | null) => { setMember(next); scroller.current?.scrollTo({ top: 0 }); };
  const cards = [
    {
      id: 'analytics-range', title: '时间范围', summary: range.from || range.to ? (range.from || '最早') + ' 至 ' + (range.to || '今天') : '全部',
      content: <><DateRange label="统计的时间范围" from={range.from} to={range.to} onChange={setRange} />
        {(range.from || range.to) && <button type="button" className="secondary-button range-clear" onClick={() => setRange({ from: '', to: '' })}>不限时间</button>}</>,
    },
    { id: 'group-export', title: '导出', summary: group?.displayName ?? '', content: <GroupExport group={group} member={member} range={range} /> },
    { id: 'analytics-excluded', title: '不统计的好友', content: <ExcludedPeople key={scope} onSaved={() => reloadAnalytics()} /> },
  ];
  const view = <ScrollPage label="聊天分析" icon={ChartBar} scroller={scroller} tabs={
    <Segmented className="settings-tabs" count={2} index={mode === 'private' ? 0 : 1} role="group" aria-label="分析哪种聊天">
      <button type="button" aria-pressed={mode === 'private'} onClick={() => setMode('private')}>私聊</button>
      <button type="button" aria-pressed={mode === 'group'} onClick={() => setMode('group')}>群聊</button>
    </Segmented>}>
    <Connected backend={shell.backend} offline="连接 weflow serve 后，可以在这里查看私聊和群聊的统计。">
      {mode === 'group'
        ? <div className="group-analytics"><GroupPicker state={groups} selected={group} onRetry={reloadGroups} onSelect={id => { setGroupId(id); setMember(null); }} />
          {group && (member
            ? <MemberView group={group} member={member} range={range} onBack={() => showMember(null)} />
            : <GroupStats group={group} range={range} onMember={showMember} />)}</div>
        : <PrivateAnalytics state={analytics} onRetry={() => reloadAnalytics()} onRecount={() => reloadAnalytics(true)}
          canChat={username => shell.sessionList.some(s => s.username === username)} onChat={chatWith} />}
    </Connected>
  </ScrollPage>;
  return { view, cards, shown: connected ? (mode === 'group' ? ['analytics-range', 'group-export', 'tasks'] : ['analytics-excluded']) : [] };
}
