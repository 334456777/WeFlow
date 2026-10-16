// A time range picked in one calendar (#224): from the start field, the first day picked is the start and the
// second the end (swapped when it comes first); from the end field, the day picked is the end. The calendar opens
// out of the fields and closes once the range is picked, on Esc, or on a click outside.
import { useRef, useState } from 'react';
import { ArrowsLeftRight, CalendarBlank } from '@phosphor-icons/react';
import { Popover } from '../Popover';
import { Calendar, openingDay } from './Calendar';
import { ordered, today } from './dates';

export interface Range { from: string; to: string }

type Side = 'from' | 'to';

export function DateRange({ from, to, onChange, onOpen, max = today(), marks, label = '时间范围' }: {
  from: string;
  to: string;
  onChange: (range: Range) => void;
  /** Called as the calendar opens, to ask for its marks. */
  onOpen?: () => void;
  max?: string;
  marks?: Record<string, number> | null;
  label?: string;
}) {
  const [side, setSide] = useState<Side | null>(null);
  // the start picked from the start field while its end is still to pick
  const [pending, setPending] = useState<string | null>(null);
  // the side shown while the calendar plays its exit
  const shown = useRef<Side>('from');
  if (side) shown.current = side;
  const anchor = useRef<HTMLDivElement>(null);
  const fields = { from: useRef<HTMLButtonElement>(null), to: useRef<HTMLButtonElement>(null) };

  const close = (focus = true) => {
    if (focus && side) fields[side].current?.focus();
    setSide(null); setPending(null);
  };
  const toggle = (which: Side) => {
    setPending(null);
    if (side === null) onOpen?.();
    setSide(current => (current === which ? null : which));
  };

  const pick = (day: string) => {
    if (side === 'to') {
      const [start, end] = from ? ordered(from, day) : [day, day];
      onChange({ from: start, to: end });
      close();
    } else if (pending === null) {
      // the start; until the end is picked, the range is at least that day
      onChange({ from: day, to: to && to >= day ? to : day });
      setPending(day);
    } else {
      const [start, end] = ordered(pending, day);
      onChange({ from: start, to: end });
      close();
    }
  };

  const field = (which: Side, value: string, name: string) =>
    <button ref={fields[which]} type="button" className={'date-field' + (side === which ? ' open' : '')}
      aria-label={name + '：' + (value || '未选择')} aria-haspopup="dialog" aria-expanded={side === which}
      onClick={() => toggle(which)}>
      <CalendarBlank size={16} aria-hidden="true" /><span className="date-display">{value || '选择日期'}</span>
    </button>;

  return <div ref={anchor} className="date-range">
    {field('from', from, '开始日期')}
    <ArrowsLeftRight className="range-arrow" size={16} aria-hidden="true" />
    {field('to', to, '结束日期')}
    <Popover anchor={anchor} open={side !== null} onClose={reason => close(reason === 'escape')}
      label={label + '：' + (shown.current === 'to' || pending ? '选择结束日期' : '选择开始日期')} className="calendar-popover">
      <Calendar key={shown.current} from={from} to={to} pending={pending} max={max} marks={marks}
        initial={openingDay(shown.current === 'to' ? to || from : from || to, max)} onPick={pick} />
    </Popover>
  </div>;
}
