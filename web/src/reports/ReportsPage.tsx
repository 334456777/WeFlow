// 报告页（#208）：有消息的年份，最近的在前；下面是选中那一年的年度报告，或者那一年和一位好友的双人报告。
import { getReportYears } from '../api/reports';
import { Chip } from '../Chip';
import { ListSkeleton } from '../Skeleton';
import { useRead } from '../useRead';
import { Failed } from '../Failed';
import { AnnualReportPage } from './AnnualReport';
import { DualReportPage, type Friend } from './DualReport';
import { t } from '../i18n';

/** 可选的年份和选中那一年的报告，`year` 为 null 时是最近的一年；`onShown` 收到当前显示的报告，没有时为 null。 */
export function ReportsPage({ mode, year, onYear, friend, onFriend, onShown }: {
  mode: 'annual' | 'dual'; year: number | null; onYear(year: number): void; friend: Friend | null; onFriend(friend: Friend): void;
  onShown(key: string | null): void;
}) {
  const [years, retry] = useRead('years', signal => getReportYears(signal));
  if (years.status === 'error') return <Failed title={t('读取年份失败')} message={years.message} onRetry={retry} />;
  if (years.status === 'loading') return <ListSkeleton label={t('正在读取有消息的年份')} rows={1} />;
  const list = [...years.value.years].sort((a, b) => b - a);
  if (!list.length) return <p className="muted report-years">{t('还没有可以生成报告的私聊消息。')}</p>;
  const chosen = year ?? list[0];
  return <>
    <div className="choice-chips report-years" role="group" aria-label={t('报告的年份')}>
      {list.map(y => <Chip key={y} label={t('{y} 年', { y })} pressed={chosen === y} onClick={() => onYear(y)} />)}
      <Chip label={t('全部年份')} pressed={chosen === 0} onClick={() => onYear(0)} />
    </div>
    {mode === 'annual' ? <AnnualReportPage year={chosen} onShown={onShown} /> : <DualReportPage friend={friend} onFriend={onFriend} year={chosen} onShown={onShown} />}
  </>;
}
