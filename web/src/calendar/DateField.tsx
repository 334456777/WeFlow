// One day picked in the calendar (#199: the day a conversation jumps to). Like DateRange, the calendar grows out of
// the field and closes once a day is picked, on Esc, or on a click outside.
import { useRef, useState } from 'react';
import { CalendarBlank } from '@phosphor-icons/react';
import { Popover } from '../Popover';
import { Calendar, openingDay } from './Calendar';
import { today } from './dates';

export function DateField({ value, onChange, onOpen, max = today(), marks, label, placeholder = '选择日期' }: {
  value: string;
  onChange: (day: string) => void;
  /** Called as the calendar opens, to ask for its marks. */
  onOpen?: () => void;
  max?: string;
  marks?: Record<string, number> | null;
  label: string;
  placeholder?: string;
}) {
  const [open, setOpen] = useState(false);
  const field = useRef<HTMLButtonElement>(null);
  const close = (focus: boolean) => { if (focus) field.current?.focus(); setOpen(false); };
  return <>
    <button ref={field} type="button" className={'date-field' + (open ? ' open' : '')} aria-label={label + '：' + (value || '未选择')}
      aria-haspopup="dialog" aria-expanded={open} onClick={() => { if (!open) onOpen?.(); setOpen(!open); }}>
      <CalendarBlank size={16} aria-hidden="true" /><span className="date-display">{value || placeholder}</span>
    </button>
    <Popover anchor={field} open={open} onClose={reason => close(reason === 'escape')} label={label} className="calendar-popover">
      <Calendar from={value} to={value} max={max} marks={marks} initial={openingDay(value, max)} onPick={day => { onChange(day); close(true); }} />
    </Popover>
  </>;
}
