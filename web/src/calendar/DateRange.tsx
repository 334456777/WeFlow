// 在一个日历里选时间范围（#224）：从开始日期框打开时，第一次选的是开始、第二次选的是结束（第二次选的更早时两者互换）；
// 从结束日期框打开时，选的那一天是结束。日历从日期框处展开，选完范围、按 Esc 或点击外面时关闭。
import { useRef, useState } from 'react';
import { ArrowsLeftRight, CalendarBlank } from '@phosphor-icons/react';
import { Popover } from '../Popover';
import { Calendar, openingDay } from './Calendar';
import { ordered, today } from './dates';
import { t } from '../i18n';

/** 一段日期，写成 `YYYY-MM-DD`，两端都包含在内；为空表示这一端不限。 */
export interface Range { from: string; to: string }

type Side = 'from' | 'to';

/** 范围的开始、结束两个日期框，以及它们共用的日历。 */
export function DateRange({ from, to, onChange, onOpen, max = today(), marks, label = t('时间范围') }: {
  from: string;
  to: string;
  onChange: (range: Range) => void;
  /** 日历打开时调用，用来请求要标出的日子（`marks`）。 */
  onOpen?: () => void;
  max?: string;
  marks?: Record<string, number> | null;
  label?: string;
}) {
  const [side, setSide] = useState<Side | null>(null);
  // 从开始日期框选了开始、还没有选结束时的开始日期
  const [pending, setPending] = useState<string | null>(null);
  // 日历播放退出动画期间仍按这一端显示
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
      // 选的是开始；选结束之前，范围至少包含这一天
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
      aria-label={t('{label}：{value}', { label: name, value: value || t('未选择') })} aria-haspopup="dialog" aria-expanded={side === which}
      onClick={() => toggle(which)}>
      <CalendarBlank size={16} aria-hidden="true" /><span className="date-display">{value || t('选择日期')}</span>
    </button>;

  return <div ref={anchor} className="date-range">
    {field('from', from, t('开始日期'))}
    <ArrowsLeftRight className="range-arrow" size={16} aria-hidden="true" />
    {field('to', to, t('结束日期'))}
    <Popover anchor={anchor} open={side !== null} onClose={reason => close(reason === 'escape')}
      label={t('{label}：{value}', { label, value: shown.current === 'to' || pending ? t('选择结束日期') : t('选择开始日期') })} className="calendar-popover">
      <Calendar key={shown.current} range from={from} to={to} pending={pending} max={max} marks={marks}
        initial={openingDay(shown.current === 'to' ? to || from : from || to, max)} onPick={pick} />
    </Popover>
  </div>;
}
