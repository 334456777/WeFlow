// The Moments page (#202): who posts beside the timeline, narrowed to a poster, words and days by its console card.
// Not connected, the sample is shown in the chat page's browser instead.
import { useEffect, useRef, useState } from 'react';
import { NO_FILTER, type SnsFilter } from '../api/sns';
import type { Page, Shell } from '../shell/page';
import { BrowserPanel } from '../shell/panels';
import { TokenPrompt } from '../TokenForm';
import { filterSummary, MomentsFilter, MomentsStats } from './MomentsCards';
import { MomentPeople } from './MomentPeople';
import { MomentsFeed } from './MomentsFeed';
import { useMomentPeople } from './useMomentPeople';

/** `active`: the page browsed, the one the console keeps the cards of while the settings are open. */
export function useMomentsPage(shell: Shell, active: boolean): Page {
  const { backend, scope, setMobileDetail } = shell;
  const live = active && shell.connected;
  const [filter, setFilter] = useState<SnsFilter>(NO_FILTER);
  useEffect(() => { setFilter(NO_FILTER); }, [scope]);
  const [people, reloadPeople] = useMomentPeople(live, scope, shell.sessionList[0]?.selfWxid);
  const pane = useRef<HTMLDivElement>(null);
  const cards = [
    {
      id: 'moments-filter', title: '筛选朋友圈', summary: filterSummary(filter),
      content: <MomentsFilter key={scope} filter={filter} onChange={setFilter} />,
    },
    {
      id: 'moments-stats', title: '朋友圈统计',
      content: (open: boolean) => <MomentsStats scope={scope} active={open && live} />,
    },
  ];
  if (!live && !(active && backend.state === 'unauthorized')) return { cards };
  const view = <BrowserPanel label="朋友圈浏览" detail={shell.mobileDetail} onBack={() => setMobileDetail(false)} back="返回发布者列表" title="朋友圈" paneLabel="朋友圈时间线" pane={pane}
    list={backend.state === 'unauthorized'
      ? <TokenPrompt />
      : <MomentPeople state={people} total={null} selected={filter.username} onRetry={reloadPeople}
        onSelect={username => { setFilter(f => ({ ...f, username })); setMobileDetail(true); }} />}>
    {live && <MomentsFeed key={scope + ':' + JSON.stringify(filter)} filter={filter} scroller={pane} onPreview={shell.openPreview} />}
  </BrowserPanel>;
  return { view, cards, shown: live ? cards.map(card => card.id) : undefined };
}
