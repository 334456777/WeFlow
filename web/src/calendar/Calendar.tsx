// 日期选择器里的日历（#224）：按 ARIA 的网格（grid）模式排列一个月的日子，周一在前；另有选月份和选年份的视图，
// 用来跳到很早的记录。`max` 之后的日子不能选。
import { useEffect, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { CaretLeft, CaretRight } from '@phosphor-icons/react';
import {
  addDays, addMonths, clampDay, dayLabel, endOfWeek, monthGrid, monthOf, ordered, startOfWeek, today, toDay, weekdays,
} from './dates';
import { formatDate, t } from '../i18n';

type View = 'days' | 'months' | 'years';

/** 日历的属性：显示什么、选中一天时做什么。 */
export interface CalendarProps {
  /** 选中的日子：一天（`from` = `to`）、一段范围，或者没有（''）。 */
  from: string;
  to: string;
  /** 按范围选日子：同时选中多天。 */
  range?: boolean;
  /** 正在选结束日期的范围的起点：预览从它到指针所在日子的范围。 */
  pending?: string | null;
  /** 日历打开时聚焦的日子。 */
  initial: string;
  max?: string;
  /** 有消息的日子和消息的条数（`chat date-counts`）：用圆点标出，其他日子的数字颜色较淡。 */
  marks?: Record<string, number> | null;
  onPick: (day: string) => void;
}

const YEARS = 12;

/** 一个月的日子，或者选月份、选年份的视图；选中一天时以这一天调用 `onPick`。 */
export function Calendar({ from, to, range, pending, initial, max, marks, onPick }: CalendarProps) {
  const [focus, setFocus] = useState(() => clampDay(initial, max));
  const [view, setView] = useState<View>('days');
  const [hover, setHover] = useState<string | null>(null);
  const grid = useRef<HTMLTableElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const moved = useRef(true);
  const opened = useRef(false);
  const now = today();
  const { year, month } = monthOf(focus);
  const maxMonth = max ? monthOf(max) : null;

  // 用键盘移动了聚焦的日子时、以及日历打开时，让这一天获得键盘焦点
  useEffect(() => {
    if (view !== 'days' || !moved.current) return;
    moved.current = false;
    grid.current?.querySelector<HTMLElement>('[data-day="' + focus + '"]')?.focus();
  }, [focus, view]);

  // 月份或年份的列表打开时，焦点放到选中的那一项上（打开它的控件已经不在了），
  // 这样键盘从那里接着操作，按 Esc 也仍能关闭弹出层
  useEffect(() => {
    if (!opened.current) { opened.current = true; return; }
    if (view === 'days') return;
    const choices = root.current?.querySelectorAll<HTMLButtonElement>('.calendar-choice:not(:disabled)');
    const chosen = [...(choices ?? [])].find(choice => choice.classList.contains('chosen'));
    (chosen ?? choices?.[0])?.focus();
  }, [view]);

  const move = (day: string) => { moved.current = true; setFocus(clampDay(day, max)); };
  const disabled = (day: string) => Boolean(max && day > max);

  const keyDown = (event: KeyboardEvent) => {
    const step: Record<string, () => string> = {
      ArrowLeft: () => addDays(focus, -1),
      ArrowRight: () => addDays(focus, 1),
      ArrowUp: () => addDays(focus, -7),
      ArrowDown: () => addDays(focus, 7),
      PageUp: () => addMonths(focus, event.shiftKey ? -12 : -1),
      PageDown: () => addMonths(focus, event.shiftKey ? 12 : 1),
      Home: () => startOfWeek(focus),
      End: () => endOfWeek(focus),
    };
    if (step[event.key]) { event.preventDefault(); move(step[event.key]()); return; }
    if ((event.key === 'Enter' || event.key === ' ') && !disabled(focus)) { event.preventDefault(); onPick(focus); }
  };

  // 显示的范围：选中的范围；正在选结束日期时，是从起点到指针所在的日子
  const [start, end] = pending ? ordered(pending, hover || pending) : [from, to];
  const monthDisabled = (y: number, m: number) => Boolean(maxMonth && (y > maxMonth.year || (y === maxMonth.year && m > maxMonth.month)));
  const firstYear = year - (((year % YEARS) + YEARS) % YEARS);
  // 一天的消息多少，按对数分成三档：消息越多，圆点越大
  const most = marks ? Math.max(1, ...Object.values(marks)) : 1;
  const heat = (count: number) => Math.max(1, Math.ceil((3 * Math.log1p(count)) / Math.log1p(most)));

  const shift = (months: number) => { setFocus(clampDay(addMonths(focus, months), max)); };
  const header = (title: string, onTitle: (() => void) | null, back: () => void, forward: (() => void) | null, [before, after]: [string, string]) =>
    <div className="calendar-head">
      <button type="button" className="icon-button small" aria-label={before} onClick={back}><CaretLeft size={14} /></button>
      {onTitle
        ? <button type="button" className="calendar-title as-row" aria-live="polite" onClick={onTitle}>{title}</button>
        : <span className="calendar-title" aria-live="polite">{title}</span>}
      <button type="button" className="icon-button small" aria-label={after} disabled={!forward} onClick={forward ?? undefined}><CaretRight size={14} /></button>
    </div>;

  if (view === 'years') {
    return <div ref={root} className="calendar">
      {header(firstYear + ' – ' + (firstYear + YEARS - 1), null, () => shift(-12 * YEARS),
        maxMonth && firstYear + YEARS > maxMonth.year ? null : () => shift(12 * YEARS), [t('上一组年份'), t('下一组年份')])}
      <div className="calendar-choices">
        {Array.from({ length: YEARS }, (_, i) => firstYear + i).map(y =>
          <button type="button" key={y} className={'calendar-choice as-row' + (y === year ? ' chosen' : '')} aria-pressed={y === year}
            disabled={monthDisabled(y, 0)} onClick={() => { setFocus(clampDay(addMonths(focus, (y - year) * 12), max)); setView('months'); }}>{y}</button>)}
      </div>
    </div>;
  }

  if (view === 'months') {
    return <div ref={root} className="calendar">
      {header(formatDate(new Date(year, 0, 1), { year: 'numeric' }), () => setView('years'), () => shift(-12),
        maxMonth && year >= maxMonth.year ? null : () => shift(12), [t('上一年'), t('下一年')])}
      <div className="calendar-choices">
        {Array.from({ length: 12 }, (_, m) => m).map(m =>
          <button type="button" key={m} className={'calendar-choice as-row' + (m === month ? ' chosen' : '')} aria-pressed={m === month}
            disabled={monthDisabled(year, m)} onClick={() => { move(addMonths(focus, m - month)); setView('days'); }}>{formatDate(new Date(2000, m, 1), { month: 'short' })}</button>)}
      </div>
    </div>;
  }

  const monthTitle = formatDate(new Date(year, month, 1), { year: 'numeric', month: 'long' });
  return <div ref={root} className="calendar">
    {header(monthTitle, () => setView('months'), () => shift(-1),
      monthDisabled(year, month + 1) ? null : () => shift(1), [t('上一个月'), t('下一个月')])}
    <table ref={grid} role="grid" className="calendar-grid" aria-label={monthTitle}
      aria-multiselectable={range || undefined} onKeyDown={keyDown}
      onPointerLeave={() => setHover(null)}>
      <thead><tr>{weekdays().map(name => <th key={name.long} scope="col" abbr={name.long}>{name.narrow}</th>)}</tr></thead>
      <tbody>{monthGrid(year, month).map(week => <tr key={week[0]}>{week.map(day => {
        if (monthOf(day).month !== month) return <td key={day} role="gridcell" className="calendar-blank" />;
        const off = disabled(day);
        const ends = Boolean(start) && (day === start || day === end);
        const between = Boolean(start && end) && day > start && day < end;
        const count = marks?.[day] ?? 0;
        const className = 'calendar-day as-row' + (ends ? ' chosen' : '') + (between ? ' between' : '') + (day === now ? ' today' : '')
          + (marks ? (count ? ' marked' : ' quiet') : '');
        return <td key={day} role="gridcell" data-day={day} data-heat={count ? heat(count) : undefined} className={className} tabIndex={day === focus ? 0 : -1}
          aria-selected={ends || between} aria-disabled={off || undefined} aria-current={day === now ? 'date' : undefined}
          aria-label={count ? t('{day}，{n} 条消息', { day: dayLabel(day), n: count }) : dayLabel(day)}
          onClick={() => { if (!off) { setFocus(day); onPick(day); } }} onPointerEnter={() => setHover(day)}>
          <span aria-hidden="true">{Number(day.slice(8))}</span>
        </td>;
      })}</tr>)}</tbody>
    </table>
  </div>;
}

/** 日历打开时显示的日子：选中的那一天，没有时是今天；都不超过 `max`。 */
export const openingDay = (day: string, max?: string) => clampDay(day || toDay(new Date()), max);
