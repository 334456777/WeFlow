// 在日历里选一天（#199：会话跳转到的那一天）。与 `DateRange` 一样，日历从日期框处展开，
// 选中一天、按 Esc 或点击外面时关闭。
import { useRef, useState } from 'react';
import { CalendarBlank } from '@phosphor-icons/react';
import { Popover } from '../Popover';
import { Calendar, openingDay } from './Calendar';
import { today } from './dates';
import { t } from '../i18n';

/** 选一天的日期框和它的日历。 */
export function DateField({ value, onChange, onOpen, max = today(), marks, label, placeholder = t('选择日期') }: {
  value: string;
  onChange: (day: string) => void;
  /** 日历打开时调用，用来请求要标出的日子（`marks`）。 */
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
    <button ref={field} type="button" className={'date-field' + (open ? ' open' : '')} aria-label={t('{label}：{value}', { label, value: value || t('未选择') })}
      aria-haspopup="dialog" aria-expanded={open} onClick={() => { if (!open) onOpen?.(); setOpen(!open); }}>
      <CalendarBlank size={16} aria-hidden="true" /><span className="date-display">{value || placeholder}</span>
    </button>
    <Popover anchor={field} open={open} onClose={reason => close(reason === 'escape')} label={label} className="calendar-popover">
      <Calendar from={value} to={value} max={max} marks={marks} initial={openingDay(value, max)} onPick={day => { onChange(day); close(true); }} />
    </Popover>
  </>;
}
