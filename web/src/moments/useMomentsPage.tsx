// 朋友圈页（#202）：时间线旁边是发布者列表；可以只看一个发布者，或者用操作控制台的卡片按文字和日期筛选。
// 朋友圈来自服务端，还没有服务端回答时来自示例数据（#879）。
import { useEffect, useRef, useState } from 'react';
import { NO_FILTER, type SnsFilter } from '../api/sns';
import type { Page, Shell } from '../shell/page';
import { BrowserPanel } from '../shell/panels';
import { TokenPrompt } from '../TokenForm';
import { filterSummary, MomentsFilter, MomentsStats } from './MomentsCards';
import { MomentPeople } from './MomentPeople';
import { MomentsFeed } from './MomentsFeed';
import { useMomentPeople } from './useMomentPeople';
import { t } from '../i18n';
import './moments.css';

/** 朋友圈页：页面视图和操作控制台的卡片。`active` 表示正在浏览这一页，打开设置时操作控制台仍保留它的卡片。 */
export function useMomentsPage(shell: Shell, active: boolean): Page {
  const { backend, scope, setMobileDetail } = shell;
  const live = active && shell.records;
  const [filter, setFilter] = useState<SnsFilter>(NO_FILTER);
  useEffect(() => { setFilter(NO_FILTER); }, [scope]);
  const [people, reloadPeople] = useMomentPeople(live, scope, shell.sessionList[0]?.selfWxid);
  const pane = useRef<HTMLDivElement>(null);
  const cards = [
    {
      id: 'moments-filter', title: t('筛选朋友圈'), summary: filterSummary(filter),
      content: <MomentsFilter key={scope} filter={filter} onChange={setFilter} />,
    },
    {
      id: 'moments-stats', title: t('朋友圈统计'),
      content: (open: boolean) => <MomentsStats scope={scope} active={open && live} />,
    },
  ];
  if (!live && !(active && backend.state === 'unauthorized')) return { cards };
  const view = <BrowserPanel label={t('朋友圈浏览')} detail={shell.mobileDetail} onBack={() => setMobileDetail(false)} back={t('返回发布者列表')} title={t('朋友圈')} paneLabel={t('朋友圈时间线')} pane={pane} detailKey={filter.username}
    list={backend.state === 'unauthorized'
      ? <TokenPrompt />
      : <MomentPeople state={people} total={null} selected={filter.username} onRetry={reloadPeople}
        onSelect={username => { setFilter(f => ({ ...f, username })); setMobileDetail(true); }} />}>
    {live && <MomentsFeed key={scope + ':' + JSON.stringify(filter)} filter={filter} scroller={pane} onPreview={shell.openPreview} />}
  </BrowserPanel>;
  return { view, cards, shown: live ? cards.map(card => card.id) : undefined };
}
