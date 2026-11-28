// 报告页（#208）：选中年份（选之前是最近的一年）的年度报告，或者和一位好友的双人报告；在操作控制台里保存。
import { useEffect, useRef, useState } from 'react';
import { CalendarStar } from '@phosphor-icons/react';
import { Segmented } from '../Segmented';
import type { Page, Shell } from '../shell/page';
import { Connected, ScrollPage } from '../shell/panels';
import type { Friend } from './DualReport';
import { ReportsPage } from './ReportsPage';
import { SaveReport } from './SaveReport';
import { t } from '../i18n';
import './reports.css';

/** 报告页：页面视图和操作控制台的卡片。 */
export function useReportsPage(shell: Shell): Page {
  const { scope } = shell;
  const [mode, setMode] = useState<'annual' | 'dual'>('annual');
  const [year, setYear] = useState<number | null>(null);
  const [friend, setFriend] = useState<Friend | null>(null);
  useEffect(() => { setYear(null); setFriend(null); }, [scope]);
  // 可以保存的报告的键；正在读取、读取失败或者还在等选好友时为 null
  const [shown, setShown] = useState<string | null>(null);
  const scroller = useRef<HTMLElement>(null);
  const cards = [{ id: 'report-save', title: t('保存报告'), content: <SaveReport report={shown} find={() => scroller.current?.querySelector<HTMLElement>('article.report') ?? null} /> }];
  const view = <ScrollPage label={t('年度报告')} icon={CalendarStar} scroller={scroller} tabs={
    <Segmented className="settings-tabs" count={2} index={mode === 'annual' ? 0 : 1} role="group" aria-label={t('哪种报告')}>
      <button type="button" aria-pressed={mode === 'annual'} onClick={() => setMode('annual')}>{t('年度')}</button>
      <button type="button" aria-pressed={mode === 'dual'} onClick={() => setMode('dual')}>{t('双人')}</button>
    </Segmented>}>
    <Connected backend={shell.backend} offline={t('连接 weflow serve 后，可以在这里生成私聊的年度报告。')}>
      <ReportsPage key={scope} mode={mode} year={year} onYear={next => { setYear(next); scroller.current?.scrollTo({ top: 0 }); }}
        friend={friend} onFriend={setFriend} onShown={setShown} />
    </Connected>
  </ScrollPage>;
  return { view, cards, shown: shell.records ? ['report-save'] : [] };
}
