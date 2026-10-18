// The console's cards on the Moments page (#202): narrowing the timeline by words and days, and the totals.
import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { MagnifyingGlass, X } from '@phosphor-icons/react';
import { getSnsStats, type SnsFilter, type SnsStats } from '../api/sns';
import { DateRange } from '../calendar/DateRange';
import { addDays, parseDay } from '../calendar/dates';

/** Unix seconds of the start of a local day, and of the last second of one. */
const startOf = (day: string) => (day ? parseDay(day).getTime() / 1000 : 0);
const endOf = (day: string) => (day ? parseDay(addDays(day, 1)).getTime() / 1000 - 1 : 0);

export function filterSummary(filter: SnsFilter): string {
  const parts = [filter.keyword.trim() && '「' + filter.keyword.trim() + '」', (filter.start || filter.end) && '按日期'].filter(Boolean);
  return parts.length ? parts.join(' · ') : '全部';
}

/** Words and days, applied when sent; the poster is picked in the list. */
export function MomentsFilter({ filter, onChange }: { filter: SnsFilter; onChange(filter: SnsFilter): void }) {
  const [keyword, setKeyword] = useState(filter.keyword);
  const [range, setRange] = useState({ from: '', to: '' });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    onChange({ ...filter, keyword: keyword.trim(), start: startOf(range.from), end: endOf(range.to) });
  };
  const narrowed = Boolean(filter.keyword || filter.start || filter.end);
  return <form className="moments-filter" onSubmit={submit}>
    <div className="search-form">
      <div className="search-input"><MagnifyingGlass size={16} aria-hidden="true" />
        <input type="search" aria-label="朋友圈里的文字" placeholder="朋友圈里的文字" value={keyword} onChange={event => setKeyword(event.target.value)} />
      </div>
      <button className="primary-button search-button" type="submit"><MagnifyingGlass size={16} /><span className="search-button-label">筛选</span></button>
    </div>
    <div className="search-date-row"><span className="field-label">时间范围</span>
      <DateRange label="朋友圈的时间范围" from={range.from} to={range.to} onChange={setRange} /></div>
    {narrowed && <button type="button" className="text-button moments-clear" onClick={() => { setKeyword(''); setRange({ from: '', to: '' }); onChange({ ...filter, keyword: '', start: 0, end: 0 }); }}>
      <X size={12} />清除筛选</button>}
  </form>;
}

/** The totals of `sns stats`, read while the card is open. */
export function MomentsStats({ scope, active }: { scope: number; active: boolean }) {
  const [stats, setStats] = useState<{ scope: number; value: SnsStats } | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!active) return;
    const controller = new AbortController();
    setFailed(false);
    getSnsStats(controller.signal).then(value => setStats({ scope, value }), () => { if (!controller.signal.aborted) setFailed(true); });
    return () => controller.abort();
  }, [scope, active]);
  const value = stats?.scope === scope ? stats.value : null;
  if (failed && !value) return <p className="empty-text" role="alert">读取朋友圈统计失败。</p>;
  if (!value) return <p className="empty-text" aria-busy="true">正在统计…</p>;
  return <dl className="info-grid">
    <div><dt>朋友圈</dt><dd className="num">{value.totalPosts.toLocaleString('zh-CN')} 条</dd></div>
    <div><dt>发布者</dt><dd className="num">{value.totalFriends.toLocaleString('zh-CN')} 人</dd></div>
    {value.myPosts !== null && <div><dt>我发的</dt><dd className="num">{value.myPosts.toLocaleString('zh-CN')} 条</dd></div>}
  </dl>;
}
