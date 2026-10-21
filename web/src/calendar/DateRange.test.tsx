import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from '#test';
import { DateField } from './DateField';
import { DateRange, type Range } from './DateRange';

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(2026, 9, 16, 10));
});
afterEach(() => { cleanup(); vi.useRealTimers(); });

function Harness({ initial, marks, changes }: { initial: Range; marks?: Record<string, number>; changes: Range[] }) {
  const [range, setRange] = useState(initial);
  return <><DateRange from={range.from} to={range.to} marks={marks} onChange={next => { changes.push(next); setRange(next); }} /><button type="button">外面</button></>;
}

const day = (n: number) => screen.getByRole('gridcell', { name: new RegExp('^2026年10月' + n + '日') });
const cell = (name: string) => screen.getByRole('gridcell', { name: new RegExp('^' + name) });
const open = (name: string) => fireEvent.click(screen.getByRole('button', { name: new RegExp('^' + name) }));

describe('DateRange', () => {
  it('picks the start, then the end from one calendar, and closes', async () => {
    const changes: Range[] = [];
    render(<Harness initial={{ from: '2026-10-01', to: '2026-10-12' }} changes={changes} />);
    open('开始日期');
    expect(screen.getByRole('grid', { name: '2026年10月' })).toBeTruthy();
    // the calendar opens on the start, with the keyboard on it
    expect(document.activeElement).toBe(day(1));
    expect(day(1).getAttribute('aria-selected')).toBe('true');
    expect(day(5).getAttribute('aria-selected')).toBe('true');

    fireEvent.click(day(8));
    expect(changes.at(-1)).toEqual({ from: '2026-10-08', to: '2026-10-12' });
    // the range up to the day under the pointer is previewed
    fireEvent.pointerEnter(day(10));
    expect(day(9).className).toContain('between');
    expect(day(11).className).not.toContain('between');
    // an end before the start: the two swap
    fireEvent.click(day(3));
    expect(changes.at(-1)).toEqual({ from: '2026-10-03', to: '2026-10-08' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(screen.getByRole('button', { name: '开始日期：2026-10-03' })).toBeTruthy();
  });

  it('picks only the end from the end field, and never a day after today', async () => {
    const changes: Range[] = [];
    render(<Harness initial={{ from: '2026-10-05', to: '2026-10-12' }} changes={changes} />);
    open('结束日期');
    expect(document.activeElement).toBe(day(12));
    expect(day(17).getAttribute('aria-disabled')).toBe('true');
    fireEvent.click(day(17));
    expect(changes).toHaveLength(0);
    expect(screen.getByRole('button', { name: '下一个月' })).toHaveProperty('disabled', true);
    expect(day(16).getAttribute('aria-current')).toBe('date');
    fireEvent.click(day(2));
    expect(changes.at(-1)).toEqual({ from: '2026-10-02', to: '2026-10-05' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('works from the keyboard and gives the focus back on Esc', async () => {
    const changes: Range[] = [];
    render(<Harness initial={{ from: '2026-10-01', to: '2026-10-09' }} changes={changes} />);
    open('开始日期');
    const grid = screen.getByRole('grid');
    fireEvent.keyDown(grid, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(day(8));
    fireEvent.keyDown(grid, { key: 'End' });
    expect(document.activeElement).toBe(day(11));
    fireEvent.keyDown(grid, { key: 'ArrowRight' });
    expect(document.activeElement).toBe(day(12));
    // not past today
    fireEvent.keyDown(grid, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(day(16));
    fireEvent.keyDown(grid, { key: 'PageUp' });
    expect(screen.getByRole('grid', { name: '2026年9月' })).toBeTruthy();
    fireEvent.keyDown(screen.getByRole('grid'), { key: 'Home' });
    fireEvent.keyDown(screen.getByRole('grid'), { key: 'Enter' });
    expect(changes.at(-1)).toEqual({ from: '2026-09-14', to: '2026-10-09' });
    fireEvent.keyDown(screen.getByRole('grid'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(document.activeElement).toBe(screen.getByRole('button', { name: /^开始日期/ }));
  });

  it('closes on a click outside, and jumps by month and year', async () => {
    render(<Harness initial={{ from: '2026-10-07', to: '2026-10-09' }} changes={[]} />);
    open('开始日期');
    fireEvent.click(screen.getByRole('button', { name: '2026年10月' }));
    fireEvent.click(screen.getByRole('button', { name: '2026年' }));
    fireEvent.click(screen.getByRole('button', { name: '2019' }));
    fireEvent.click(screen.getByRole('button', { name: '3月' }));
    expect(screen.getByRole('grid', { name: '2019年3月' })).toBeTruthy();
    act(() => { fireEvent.pointerDown(screen.getByRole('button', { name: '外面' })); });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('keeps the keyboard in the month and year lists, where Esc still closes (#238)', async () => {
    render(<Harness initial={{ from: '2026-10-07', to: '2026-10-09' }} changes={[]} />);
    const field = screen.getByRole('button', { name: /^开始日期/ });
    open('开始日期');
    fireEvent.click(screen.getByRole('button', { name: '2026年10月' }));
    expect(document.activeElement).toBe(screen.getByRole('button', { name: '10月' }));
    // the year list has no title to click: the focus goes to the year chosen, not to <body>
    fireEvent.click(screen.getByRole('button', { name: '2026年' }));
    expect(document.activeElement).toBe(screen.getByRole('button', { name: '2026' }));
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(document.activeElement).toBe(field);

    // a year picked: the month list takes the focus, and a month the day grid
    open('开始日期');
    fireEvent.click(screen.getByRole('button', { name: '2026年10月' }));
    fireEvent.click(screen.getByRole('button', { name: '2026年' }));
    fireEvent.click(screen.getByRole('button', { name: '2019' }));
    expect(document.activeElement).toBe(screen.getByRole('button', { name: '10月' }));
    fireEvent.click(screen.getByRole('button', { name: '3月' }));
    expect(document.activeElement).toBe(cell('2019年3月7日'));
    // with the focus lost all the same (on <body>), Esc closes and gives the focus back
    (document.activeElement as HTMLElement).blur();
    expect(document.activeElement).toBe(document.body);
    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(document.activeElement).toBe(field);
  });

  it('tells that a range is several days, and one day is one (#240)', () => {
    render(<Harness initial={{ from: '2026-10-01', to: '2026-10-12' }} changes={[]} />);
    open('开始日期');
    expect(screen.getByRole('grid').getAttribute('aria-multiselectable')).toBe('true');
    expect(screen.getAllByRole('gridcell', { selected: true })).toHaveLength(12);
    cleanup();
    render(<DateField value="2026-10-05" onChange={() => {}} label="跳转日期" />);
    fireEvent.click(screen.getByRole('button', { name: /^跳转日期/ }));
    expect(screen.getByRole('grid').getAttribute('aria-multiselectable')).toBeNull();
    expect(screen.getAllByRole('gridcell', { selected: true })).toHaveLength(1);
  });

  it('marks the days with messages and quiets the others', () => {
    render(<Harness initial={{ from: '2026-10-07', to: '2026-10-09' }} marks={{ '2026-10-03': 12 }} changes={[]} />);
    open('开始日期');
    expect(day(3).className).toContain('marked');
    expect(day(3).getAttribute('aria-label')).toContain('12 条消息');
    expect(day(4).className).toContain('quiet');
  });
});
