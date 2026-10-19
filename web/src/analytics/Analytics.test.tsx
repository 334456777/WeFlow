import { act, cleanup, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { hourBars, monthBars, typeBars, weekdayBars, type Overall, type TimeSpread } from '../api/analytics';
import { ExcludedPeople, PrivateAnalytics, yearStarts } from './AnalyticsViews';
import { useAnalytics, type AnalyticsState } from './useAnalytics';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const OVERALL: Overall = {
  totalMessages: 1000, textMessages: 700, imageMessages: 200, voiceMessages: 0, videoMessages: 0, emojiMessages: 60, otherMessages: 40,
  sentMessages: 400, receivedMessages: 600, firstMessageTime: 1_700_000_000, lastMessageTime: 1_760_000_000, activeDays: 60,
};
const SPREAD: TimeSpread = {
  hourlyDistribution: { 9: 100, 21: 600, 23: 300 },
  weekdayDistribution: { 1: 10, 7: 990 },
  monthlyDistribution: { '2024-11': 400, '2025-02': 600 },
};
const READY: AnalyticsState = {
  status: 'ready', overall: OVERALL, spread: SPREAD,
  rankings: [
    { username: 'wxid_a', displayName: '林沐', messageCount: 600, sentCount: 250, receivedCount: 350, lastMessageTime: 1_760_000_000 },
    { username: 'wxid_b', displayName: '陈屿', messageCount: 400, sentCount: 150, receivedCount: 250, lastMessageTime: 1_750_000_000 },
  ],
};

describe('chat analytics', () => {
  it('lays out the hours, the weekdays from Monday, every month between the first and last, and the kinds there are', () => {
    expect(hourBars(SPREAD.hourlyDistribution)).toHaveLength(24);
    expect(hourBars(SPREAD.hourlyDistribution)[21]).toEqual({ label: '21 点', value: 600 });
    expect(weekdayBars(SPREAD).map(b => b.value)).toEqual([10, 0, 0, 0, 0, 0, 990]);
    expect(weekdayBars(SPREAD)[6].label).toBe('周日');
    expect(monthBars(SPREAD)).toEqual([
      { label: '2024 年 11 月', value: 400 }, { label: '2024 年 12 月', value: 0 }, { label: '2025 年 1 月', value: 0 }, { label: '2025 年 2 月', value: 600 },
    ]);
    expect(monthBars({ ...SPREAD, monthlyDistribution: {} })).toEqual([]);
    expect(typeBars(OVERALL).map(b => b.label)).toEqual(['文字', '图片', '表情', '其他']);
    // the first month is named only when the next January is far enough away not to run into it
    const fromMarch = monthBars({ ...SPREAD, monthlyDistribution: { '2024-03': 1, '2025-02': 1 } });
    expect(fromMarch.map((_, i) => yearStarts(fromMarch, i))).toEqual(fromMarch.map(b => b.label === '2024 年 3 月' || b.label === '2025 年 1 月'));
    // over seven years an eighth is 11 months: May is too close to the January after it
    const long = monthBars({ ...SPREAD, monthlyDistribution: { '2019-05': 1, '2026-10': 1 } });
    expect(yearStarts(long, 0)).toBe(false);
    expect(long.filter((_, i) => yearStarts(long, i))).toHaveLength(7);
  });

  it('counts the chats first and reads the rankings and hours from what it kept, counting again only when asked', async () => {
    const urls: string[] = [];
    let release: () => void = () => {};
    const first = new Promise<void>(resolve => { release = resolve; });
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      urls.push(url);
      if (url.startsWith('/analytics/overall')) { await first; return json({ success: true, data: OVERALL }); }
      return json({ success: true, data: url.startsWith('/analytics/rankings') ? READY.rankings : SPREAD });
    }));
    const { result } = renderHook(() => useAnalytics(true, 0));
    await act(async () => {});
    expect(urls).toEqual(['/analytics/overall']);
    await act(async () => { release(); });
    expect(urls).toEqual(['/analytics/overall', '/analytics/rankings?limit=20', '/analytics/time']);
    expect(result.current[0].status).toBe('ready');
    await act(async () => { result.current[1](true); });
    expect(urls[3]).toBe('/analytics/overall?force=true');
  });

  it('shows the numbers, the charts and who the chats are with', () => {
    const onChat = vi.fn();
    render(<PrivateAnalytics state={READY} onRetry={() => {}} onRecount={() => {}} canChat={u => u === 'wxid_a'} onChat={onChat} />);
    const numbers = screen.getByLabelText('总览').textContent!;
    expect(numbers).toContain('1,000 条');
    expect(numbers).toContain('400 条 · 40%');
    expect(numbers).toContain('有消息的月份2 个');
    expect(screen.getByRole('img', { name: '每小时的消息：最多的是21 点，600 条' })).toBeTruthy();
    expect(screen.getByRole('img', { name: '星期几的消息：最多的是周日，990 条' })).toBeTruthy();
    expect(Array.from(screen.getByLabelText('每个月').querySelectorAll('.chart-ticks span'), s => s.textContent)).toEqual(['', '', '2025', '']);
    expect(screen.getByLabelText('各类消息的条数').textContent).toContain('文字700' + '70%');
    const rows = screen.getByLabelText('聊得最多的人').querySelectorAll('.rank-row');
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain('我发 250 · 对方 350');
    expect((screen.getByRole('button', { name: '查看和陈屿的聊天' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '查看和林沐的聊天' }));
    expect(onChat).toHaveBeenCalledWith('wxid_a');
  });

  it('says why the analytics cannot be read', () => {
    const onRetry = vi.fn();
    render(<PrivateAnalytics state={{ status: 'error', message: 'no message sessions found' }} onRetry={onRetry} onRecount={() => {}} canChat={() => true} onChat={() => {}} />);
    expect(screen.getByRole('alert').textContent).toContain('no message sessions found');
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(onRetry).toHaveBeenCalled();
  });

  it('lists everyone left out but only a few others, the rest found by searching', async () => {
    const people = Array.from({ length: 20 }, (_, i) => ({ username: 'wxid_' + i, displayName: '朋友' + i }));
    vi.stubGlobal('fetch', vi.fn(async (url: string) => json({ success: true, data: url === '/analytics/excluded' ? ['wxid_15', 'wxid_19'] : people })));
    render(<ExcludedPeople onSaved={() => {}} />);
    await act(async () => {});
    const names = Array.from(screen.getByRole('group', { name: '不统计的好友' }).querySelectorAll('label'), l => l.textContent);
    expect(names).toEqual(['朋友15', '朋友19', ...Array.from({ length: 8 }, (_, i) => '朋友' + i)]);
    expect(screen.getByText('还有 10 个，搜索名字或微信号找到他们。')).toBeTruthy();
  });

  it('leaves people out, those already out first, and puts everyone back with an empty list', async () => {
    const calls: { method: string; url: string; body?: unknown }[] = [];
    let saved = ['wxid_c'];
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      const method = init?.method ?? 'GET';
      calls.push({ method, url, body: init?.body ? JSON.parse(String(init.body)) : undefined });
      if (method === 'PUT') { const set = JSON.parse(String(init!.body)).set; saved = set === '' ? [] : set; return json({ success: true, data: saved }); }
      return json({ success: true, data: url === '/analytics/excluded' ? saved : [
        { username: 'wxid_a', displayName: '林沐' }, { username: 'wxid_b', displayName: '陈屿', wechatId: 'chenyu' }, { username: 'wxid_c', displayName: '文件传输助手' },
      ] });
    }));
    const onSaved = vi.fn();
    render(<ExcludedPeople onSaved={onSaved} />);
    await act(async () => {});
    const names = () => Array.from(screen.getByRole('group', { name: '不统计的好友' }).querySelectorAll('label'), l => l.textContent);
    expect(names()).toEqual(['文件传输助手', '林沐', '陈屿']);
    const save = screen.getByRole('button', { name: /^保存/ }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    fireEvent.change(screen.getByRole('searchbox', { name: '搜索私聊' }), { target: { value: 'CHENYU' } });
    expect(names()).toEqual(['陈屿']);
    fireEvent.click(screen.getByRole('checkbox', { name: '陈屿' }));
    expect(save.textContent).toBe('保存（不统计 2 人）');
    fireEvent.click(save);
    await act(async () => {});
    expect(calls.at(-1)).toEqual({ method: 'PUT', url: '/analytics/excluded', body: { set: ['wxid_c', 'wxid_b'] } });
    expect(onSaved).toHaveBeenCalledTimes(1);

    fireEvent.change(screen.getByRole('searchbox', { name: '搜索私聊' }), { target: { value: '' } });
    fireEvent.click(screen.getByRole('checkbox', { name: '陈屿' }));
    fireEvent.click(screen.getByRole('checkbox', { name: '文件传输助手' }));
    fireEvent.click(screen.getByRole('button', { name: /^保存/ }));
    await act(async () => {});
    expect(calls.at(-1)?.body).toEqual({ set: '' });
  });
});
