import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { busiestHour, formatWait, type AnnualReport } from '../api/reports';
import { AnnualReportPage, AnnualReportView } from './AnnualReport';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const heat = Array.from({ length: 7 }, () => Array(24).fill(0) as number[]);
heat[5][21] = 30;
heat[0][9] = 4;
const REPORT: AnnualReport = {
  year: 2025, totalMessages: 12345, totalFriends: 42,
  coreFriends: [{ username: 'wxid_a', displayName: '林沐', messageCount: 3000, sentCount: 1200, receivedCount: 1800 }],
  monthlyTopFriends: Array.from({ length: 12 }, (_, i) => (i === 2 ? { month: 3, displayName: '林沐', messageCount: 400 } : { month: i + 1, displayName: '暂无', messageCount: 0 })),
  peakDay: { date: '2025-03-14', messageCount: 320, topFriend: '林沐', topFriendCount: 200 },
  longestStreak: { friendName: '陈屿', days: 45, startDate: '2025-06-01', endDate: '2025-07-15' },
  activityHeatmap: { data: heat },
  midnightKing: null,
  mutualFriend: { displayName: '秋秋', sentCount: 300, receivedCount: 310, ratio: 0.97 },
  socialInitiative: { initiatedChats: 60, receivedChats: 40, initiativeRate: 60, topInitiatedFriend: '林沐', topInitiatedCount: 20 },
  responseSpeed: { avgResponseTime: 150, fastestFriend: '陈屿', fastestTime: 12 },
  topPhrases: [{ phrase: '好的', count: 100 }, { phrase: '哈哈哈', count: 50 }],
  snsStats: { totalPosts: 8, topLikers: [{ username: 'wxid_b', displayName: '陈屿', count: 6 }], topLiked: [] },
  lostFriend: null,
};

describe('annual report', () => {
  it('says how long a wait is and where the busiest hour is', () => {
    expect([formatWait(12.4), formatWait(150), formatWait(5400), formatWait(90_000)]).toEqual(['12 秒', '3 分钟', '1.5 小时', '25 小时']);
    expect(busiestHour(heat)).toEqual({ weekday: 5, hour: 21, count: 30 });
    expect(busiestHour(Array.from({ length: 7 }, () => Array(24).fill(0)))).toBeNull();
  });

  it('tells the year in numbers, leaving out the moments it has nothing for', () => {
    render(<AnnualReportView report={REPORT} />);
    expect(screen.getByLabelText('总览').textContent).toBe('2025 年，你在私聊里发出和收到了12,345条消息，和 42 位好友聊过天。');
    expect(screen.getByLabelText('聊得最多的人').textContent).toContain('我发 1,200 · 对方 1,800');
    const months = screen.getByLabelText('每个月').querySelectorAll('li');
    expect(months).toHaveLength(12);
    expect(months[2].textContent).toBe('3 月林沐400 条');
    expect(months[0].textContent).toBe('1 月没有聊天');
    expect(screen.getByRole('img', { name: '最常聊天的是周六 21 点，30 条' })).toBeTruthy();
    const moments = Array.from(screen.getByLabelText('值得记住的').querySelectorAll('.report-label'), l => l.textContent);
    expect(moments).toEqual(['最热闹的一天', '连续聊天最久', '有来有往', '由你开场', '平均回复']);
    expect(screen.getByLabelText('值得记住的').textContent).toContain('回得最快的是陈屿，平均 12 秒。');
    expect(screen.getByLabelText('常说的话').textContent).toContain('好的100');
    expect(screen.getByLabelText('朋友圈').textContent).toContain('给你点赞最多陈屿6');
    expect(Array.from(screen.getByLabelText('朋友圈').querySelectorAll('.report-label'), l => l.textContent)).toEqual(['给你点赞最多']);
  });

  it('leaves the months out of the report of all the years', () => {
    render(<AnnualReportView report={{ ...REPORT, year: 0 }} />);
    expect(screen.queryByLabelText('每个月')).toBeNull();
    expect(screen.getByLabelText('总览').textContent).toMatch(/^这些年/);
  });

  it('opens on the latest year and reads the one picked, or all of them', async () => {
    const urls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      urls.push(url);
      return json({ success: true, data: url.startsWith('/report/annual/years') ? { years: [2024, 2025] } : { ...REPORT, year: Number(new URL(url, 'http://x').searchParams.get('year')) } });
    }));
    const onYear = vi.fn();
    const { rerender } = render(<AnnualReportPage year={null} onYear={onYear} />);
    await act(async () => {});
    await act(async () => {});
    expect(urls).toEqual(['/report/annual/years', '/report/annual/generate?year=2025']);
    expect(Array.from(screen.getByRole('group', { name: '报告的年份' }).querySelectorAll('button'), b => b.textContent)).toEqual(['2025 年', '2024 年', '全部年份']);
    expect(screen.getByRole('button', { name: '2025 年' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: '全部年份' }));
    expect(onYear).toHaveBeenCalledWith(0);
    rerender(<AnnualReportPage year={0} onYear={onYear} />);
    await act(async () => {});
    expect(urls.at(-1)).toBe('/report/annual/generate?year=0');
    expect(screen.getByRole('article').getAttribute('aria-label')).toBe('全部年份的年度报告');
  });
});
