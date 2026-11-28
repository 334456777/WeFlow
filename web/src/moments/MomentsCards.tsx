// 朋友圈页的操作控制台卡片（#202）：按文字和日期筛选时间线，以及统计的总数。
import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { MagnifyingGlass, X } from '@phosphor-icons/react';
import { getSnsStats, type SnsFilter, type SnsStats } from '../api/sns';
import { DateRange } from '../calendar/DateRange';
import { addDays, parseDay } from '../calendar/dates';
import { GridSkeleton } from '../Skeleton';
import { locale, t } from '../i18n';

/** 本地时间一天开始时的 Unix 秒数，以及一天最后一秒的 Unix 秒数；没有日期时为 0。 */
const startOf = (day: string) => (day ? parseDay(day).getTime() / 1000 : 0);
const endOf = (day: string) => (day ? parseDay(addDays(day, 1)).getTime() / 1000 - 1 : 0);

/** 卡片收起时标题栏上的筛选摘要：关键词和 `按日期`，都没有时为 `全部`。 */
export function filterSummary(filter: SnsFilter): string {
  const parts = [filter.keyword.trim() && t('「{keyword}」', { keyword: filter.keyword.trim() }), (filter.start || filter.end) && t('按日期')].filter(Boolean);
  return parts.length ? parts.join(' · ') : t('全部');
}

/** 按文字和日期筛选，提交后才生效；发布者在列表里选。 */
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
        <input type="search" aria-label={t('朋友圈里的文字')} placeholder={t('朋友圈里的文字')} value={keyword} onChange={event => setKeyword(event.target.value)} />
      </div>
      <button className="primary-button search-button" type="submit"><MagnifyingGlass size={16} /><span className="search-button-label">{t('筛选')}</span></button>
    </div>
    <div className="search-date-row"><span className="field-label">{t('时间范围')}</span>
      <DateRange label={t('朋友圈的时间范围')} from={range.from} to={range.to} onChange={setRange} /></div>
    {narrowed && <button type="button" className="text-button moments-clear" onClick={() => { setKeyword(''); setRange({ from: '', to: '' }); onChange({ ...filter, keyword: '', start: 0, end: 0 }); }}>
      <X size={12} />{t('清除筛选')}</button>}
  </form>;
}

/** `sns stats` 的总数，卡片展开时读取。 */
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
  if (failed && !value) return <p className="empty-text" role="alert">{t('读取朋友圈统计失败。')}</p>;
  if (!value) return <GridSkeleton label={t('正在统计朋友圈')} cells={3} />;
  return <dl className="info-grid">
    <div><dt>{t('朋友圈')}</dt><dd className="num">{t('{totalPosts} 条', { totalPosts: value.totalPosts.toLocaleString(locale()) })}</dd></div>
    <div><dt>{t('发布者')}</dt><dd className="num">{t('{totalFriends} 人', { totalFriends: value.totalFriends.toLocaleString(locale()) })}</dd></div>
    {value.myPosts !== null && <div><dt>{t('我发的')}</dt><dd className="num">{t('{myPosts} 条', { myPosts: value.myPosts.toLocaleString(locale()) })}</dd></div>}
  </dl>;
}
