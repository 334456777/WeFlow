// The calendar of the date pickers (#224): a month of days in the ARIA grid pattern, Monday first, with views to
// pick a month or a year for records long ago. Days after `max` cannot be picked.
import { useEffect, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { CaretLeft, CaretRight } from '@phosphor-icons/react';
import {
  addDays, addMonths, clampDay, dayLabel, endOfWeek, monthGrid, monthOf, ordered, startOfWeek, today, toDay, WEEKDAYS,
} from './dates';

type View = 'days' | 'months' | 'years';

export interface CalendarProps {
  /** The chosen days: one day (`from` = `to`), a range, or none (''). */
  from: string;
  to: string;
  /** The start of a range whose end is being picked: the range up to the day under the pointer is previewed. */
  pending?: string | null;
  /** The day focused when the calendar opens. */
  initial: string;
  max?: string;
  /** Days with messages and how many (`chat date-counts`): marked with a dot; the others are quieter. */
  marks?: Record<string, number> | null;
  onPick: (day: string) => void;
}

const YEARS = 12;

export function Calendar({ from, to, pending, initial, max, marks, onPick }: CalendarProps) {
  const [focus, setFocus] = useState(() => clampDay(initial, max));
  const [view, setView] = useState<View>('days');
  const [hover, setHover] = useState<string | null>(null);
  const grid = useRef<HTMLTableElement>(null);
  const moved = useRef(true);
  const now = today();
  const { year, month } = monthOf(focus);
  const maxMonth = max ? monthOf(max) : null;

  // the focused day takes the keyboard focus when it moved by the keyboard, and when the calendar opens
  useEffect(() => {
    if (view !== 'days' || !moved.current) return;
    moved.current = false;
    grid.current?.querySelector<HTMLElement>('[data-day="' + focus + '"]')?.focus();
  }, [focus, view]);

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

  // the range shown: the one chosen, or while its end is picked, from its start to the day under the pointer
  const [start, end] = pending ? ordered(pending, hover || pending) : [from, to];
  const monthDisabled = (y: number, m: number) => Boolean(maxMonth && (y > maxMonth.year || (y === maxMonth.year && m > maxMonth.month)));
  const firstYear = year - (((year % YEARS) + YEARS) % YEARS);

  const shift = (months: number) => { setFocus(clampDay(addMonths(focus, months), max)); };
  const header = (title: string, onTitle: (() => void) | null, back: () => void, forward: (() => void) | null, unit: string) =>
    <div className="calendar-head">
      <button type="button" className="icon-button small" aria-label={'上一' + unit} onClick={back}><CaretLeft size={14} /></button>
      {onTitle
        ? <button type="button" className="calendar-title" aria-live="polite" onClick={onTitle}>{title}</button>
        : <span className="calendar-title" aria-live="polite">{title}</span>}
      <button type="button" className="icon-button small" aria-label={'下一' + unit} disabled={!forward} onClick={forward ?? undefined}><CaretRight size={14} /></button>
    </div>;

  if (view === 'years') {
    return <div className="calendar">
      {header(firstYear + ' – ' + (firstYear + YEARS - 1), null, () => shift(-12 * YEARS),
        maxMonth && firstYear + YEARS > maxMonth.year ? null : () => shift(12 * YEARS), '组年份')}
      <div className="calendar-choices">
        {Array.from({ length: YEARS }, (_, i) => firstYear + i).map(y =>
          <button type="button" key={y} className={'calendar-choice' + (y === year ? ' chosen' : '')} aria-pressed={y === year}
            disabled={monthDisabled(y, 0)} onClick={() => { setFocus(clampDay(addMonths(focus, (y - year) * 12), max)); setView('months'); }}>{y}</button>)}
      </div>
    </div>;
  }

  if (view === 'months') {
    return <div className="calendar">
      {header(year + '年', () => setView('years'), () => shift(-12),
        maxMonth && year >= maxMonth.year ? null : () => shift(12), '年')}
      <div className="calendar-choices">
        {Array.from({ length: 12 }, (_, m) => m).map(m =>
          <button type="button" key={m} className={'calendar-choice' + (m === month ? ' chosen' : '')} aria-pressed={m === month}
            disabled={monthDisabled(year, m)} onClick={() => { move(addMonths(focus, m - month)); setView('days'); }}>{m + 1}月</button>)}
      </div>
    </div>;
  }

  return <div className="calendar">
    {header(year + '年' + (month + 1) + '月', () => setView('months'), () => shift(-1),
      monthDisabled(year, month + 1) ? null : () => shift(1), '个月')}
    <table ref={grid} role="grid" className="calendar-grid" aria-label={year + '年' + (month + 1) + '月'} onKeyDown={keyDown}
      onPointerLeave={() => setHover(null)}>
      <thead><tr>{WEEKDAYS.map(name => <th key={name} scope="col" abbr={'星期' + name}>{name}</th>)}</tr></thead>
      <tbody>{monthGrid(year, month).map(week => <tr key={week[0]}>{week.map(day => {
        if (monthOf(day).month !== month) return <td key={day} role="gridcell" className="calendar-blank" />;
        const off = disabled(day);
        const ends = Boolean(start) && (day === start || day === end);
        const between = Boolean(start && end) && day > start && day < end;
        const count = marks?.[day] ?? 0;
        const className = 'calendar-day' + (ends ? ' chosen' : '') + (between ? ' between' : '') + (day === now ? ' today' : '')
          + (marks ? (count ? ' marked' : ' quiet') : '');
        return <td key={day} role="gridcell" data-day={day} className={className} tabIndex={day === focus ? 0 : -1}
          aria-selected={ends || between} aria-disabled={off || undefined} aria-current={day === now ? 'date' : undefined}
          aria-label={dayLabel(day) + (count ? '，' + count + ' 条消息' : '')}
          onClick={() => { if (!off) { setFocus(day); onPick(day); } }} onPointerEnter={() => setHover(day)}>
          <span aria-hidden="true">{Number(day.slice(8))}</span>
        </td>;
      })}</tr>)}</tbody>
    </table>
  </div>;
}

/** The day a calendar opens on: the chosen day, else today (or the latest allowed). */
export const openingDay = (day: string, max?: string) => clampDay(day || toDay(new Date()), max);
