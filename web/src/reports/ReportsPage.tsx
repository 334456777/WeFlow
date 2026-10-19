// The reports page (#208): the years with messages, the latest first, then the annual report of the year picked,
// or the dual report of it with one friend.
import { getReportYears } from '../api/reports';
import { ListSkeleton } from '../Skeleton';
import { useRead } from '../useRead';
import { AnnualReportPage, Failed } from './AnnualReport';
import { DualReportPage, type Friend } from './DualReport';

export function ReportsPage({ mode, year, onYear, friend, onFriend }: {
  mode: 'annual' | 'dual'; year: number | null; onYear(year: number): void; friend: Friend | null; onFriend(friend: Friend): void;
}) {
  const [years, retry] = useRead('years', signal => getReportYears(signal));
  if (years.status === 'error') return <Failed title="读取年份失败" message={years.message} onRetry={retry} />;
  if (years.status === 'loading') return <ListSkeleton label="正在读取有消息的年份" rows={1} />;
  const list = [...years.value.years].sort((a, b) => b - a);
  if (!list.length) return <p className="muted report-years">还没有可以生成报告的私聊消息。</p>;
  const chosen = year ?? list[0];
  return <>
    <div className="choice-chips report-years" role="group" aria-label="报告的年份">
      {list.map(y => <button type="button" key={y} className="chip" aria-pressed={chosen === y} onClick={() => onYear(y)}>{y} 年</button>)}
      <button type="button" className="chip" aria-pressed={chosen === 0} onClick={() => onYear(0)}>全部年份</button>
    </div>
    {mode === 'annual' ? <AnnualReportPage year={chosen} /> : <DualReportPage friend={friend} onFriend={onFriend} year={chosen} />}
  </>;
}
