// The reports page (#208): the annual or the dual report, of the year picked (the latest until then) and a friend;
// saved from the console.
import { useEffect, useRef, useState } from 'react';
import { CalendarStar } from '@phosphor-icons/react';
import { Segmented } from '../Segmented';
import type { Page, Shell } from '../shell/page';
import { Connected, ScrollPage } from '../shell/panels';
import type { Friend } from './DualReport';
import { ReportsPage } from './ReportsPage';
import { SaveReport } from './SaveReport';

export function useReportsPage(shell: Shell): Page {
  const { scope } = shell;
  const [mode, setMode] = useState<'annual' | 'dual'>('annual');
  const [year, setYear] = useState<number | null>(null);
  const [friend, setFriend] = useState<Friend | null>(null);
  useEffect(() => { setYear(null); setFriend(null); }, [scope]);
  const scroller = useRef<HTMLElement>(null);
  const cards = [{ id: 'report-save', title: '保存报告', content: <SaveReport find={() => scroller.current?.querySelector<HTMLElement>('article.report') ?? null} /> }];
  const view = <ScrollPage label="年度报告" icon={CalendarStar} scroller={scroller} tabs={
    <Segmented className="settings-tabs" count={2} index={mode === 'annual' ? 0 : 1} role="group" aria-label="哪种报告">
      <button type="button" aria-pressed={mode === 'annual'} onClick={() => setMode('annual')}>年度</button>
      <button type="button" aria-pressed={mode === 'dual'} onClick={() => setMode('dual')}>双人</button>
    </Segmented>}>
    <Connected backend={shell.backend} offline="连接 weflow serve 后，可以在这里生成私聊的年度报告。">
      <ReportsPage key={scope} mode={mode} year={year} onYear={next => { setYear(next); scroller.current?.scrollTo({ top: 0 }); }}
        friend={friend} onFriend={setFriend} />
    </Connected>
  </ScrollPage>;
  return { view, cards, shown: shell.connected ? ['report-save'] : [] };
}
