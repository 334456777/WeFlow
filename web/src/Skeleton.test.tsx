import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from '#test';
import { GridSkeleton, ListSkeleton, MessagesSkeleton, PaymentsSkeleton, ResultsSkeleton } from './Skeleton';

afterEach(cleanup);

describe('Skeleton', () => {
  it('takes the place of a list, named for what it waits for, with lines of varied lengths that stay put', () => {
    const { rerender } = render(<ListSkeleton label="正在读取通讯录" rows={5} note="已读 300 条消息" />);
    const list = screen.getByLabelText('正在读取通讯录');
    expect(list.getAttribute('aria-busy')).toBe('true');
    expect(list.querySelectorAll('.skeleton-row')).toHaveLength(5);
    expect(list.textContent).toBe('已读 300 条消息');
    const widths = () => Array.from(list.querySelectorAll<HTMLElement>('.skeleton.line:not(.short)'), l => l.style.width);
    const first = widths();
    expect(new Set(first).size).toBeGreaterThan(2);
    rerender(<ListSkeleton label="正在读取通讯录" rows={5} note="已读 300 条消息" />);
    expect(widths()).toEqual(first);
  });

  it('lays out messages on both sides and the facts of a card', () => {
    render(<><MessagesSkeleton label="正在读取消息" /><GridSkeleton label="正在统计" cells={3} /></>);
    const messages = screen.getByLabelText('正在读取消息');
    expect(messages.querySelectorAll('.message-row.outgoing').length).toBeGreaterThan(0);
    expect(messages.querySelectorAll('.message-row.incoming .skeleton.avatar').length).toBeGreaterThan(0);
    expect(screen.getByLabelText('正在统计').querySelectorAll('.grid-skeleton-cell')).toHaveLength(3);
  });

  it('stands in for the hits of a search and the records of WeChat Pay', () => {
    render(<><ResultsSkeleton label="正在搜索" rows={3} /><PaymentsSkeleton label="正在读取支付记录" /></>);
    expect(screen.getByLabelText('正在搜索').querySelectorAll('.result-skeleton')).toHaveLength(3);
    const pay = screen.getByLabelText('正在读取支付记录');
    expect(pay.querySelectorAll('.pay-card .skeleton.amount')).toHaveLength(4);
  });
});
