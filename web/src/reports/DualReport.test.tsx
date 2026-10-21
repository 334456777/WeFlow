import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { messageText, type DualReport } from '../api/reports';
import { DualReportView, FriendPicker } from './DualReport';
import { ReportsPage } from './ReportsPage';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const at = (y: number, m: number, d: number, h = 9) => new Date(y, m - 1, d, h, 30).getTime();
const REPORT: DualReport = {
  year: 2025, selfName: '我自己', friendUsername: 'wxid_a', friendName: '林沐',
  firstChat: { content: '你好', isSentByMe: false, createTime: at(2019, 5, 5) },
  firstChatMessages: [
    { content: '你好', isSentByMe: false, createTime: at(2019, 5, 5), localType: 1 },
    { content: '<msg><img md5="x"/></msg>', isSentByMe: true, createTime: at(2019, 5, 5, 10), localType: 3 },
  ],
  yearFirstChat: { content: '新年好', isSentByMe: true, createTime: at(2025, 1, 1, 0), localType: 1, firstThreeMessages: [{ content: '新年好', isSentByMe: true, createTime: at(2025, 1, 1, 0), localType: 1 }] },
  stats: { totalMessages: 3200, totalWords: 41000, imageCount: 120, voiceCount: 30, emojiCount: 410 },
  topPhrases: [{ phrase: '哈哈', count: 90 }], myExclusivePhrases: [{ phrase: '好嘞', count: 20 }], friendExclusivePhrases: [],
  heatmap: Array.from({ length: 7 }, (_, w) => Array.from({ length: 24 }, (_, h) => (w === 2 && h === 22 ? 50 : 1))),
  initiative: { initiated: 30, received: 10 },
  response: { avg: 95, fastest: 3, slowest: 7200, count: 400 },
  monthly: { '2025-01': 300, '2025-03': 200 },
  streak: { days: 12, startDate: '2025-02-01', endDate: '2025-02-12' },
};

describe('dual report', () => {
  it('names what a message is when it is not text', () => {
    expect(messageText({ content: 'hi', isSentByMe: true, createTime: 0, localType: 1 })).toBe('hi');
    expect(messageText({ content: '<msg/>', isSentByMe: true, createTime: 0, localType: 47 })).toBe('[表情]');
    expect(messageText({ content: '<msg/>', isSentByMe: true, createTime: 0, localType: 244813135921 })).toBe('[引用回复]');
    expect(messageText({ content: '<msg/>', isSentByMe: true, createTime: 0, localType: 10000 })).toBe('[消息]');
  });

  it('tells the two of us: how it began, the year, the moments and the words', () => {
    render(<DualReportView report={REPORT} />);
    expect(screen.getByRole('article').getAttribute('aria-label')).toBe('我自己和林沐的 2025 年双人报告');
    expect(screen.getByLabelText('总览').textContent).toContain('2025 年，你和林沐聊了3,200条消息，写下 41,000 个字');
    const first = screen.getByLabelText('最早的几条消息').querySelectorAll('li');
    expect(Array.from(first, li => li.className)).toEqual(['', 'mine']);
    expect(first[1].textContent).toMatch(/^我自己 · 2019\/5\/5.*\[图片\]$/);
    expect(screen.getByLabelText('2025 年最早的几条消息').textContent).toContain('新年好');
    const moments = screen.getByLabelText('值得记住的').textContent!;
    expect(moments).toContain('由你开场75%');
    expect(moments).toContain('最快 3 秒，最慢 2 小时，共 400 次回复。');
    expect(screen.getByRole('img', { name: '最常聊天的是周三 22 点，50 条' })).toBeTruthy();
    expect(Array.from(screen.getByLabelText('每个月').querySelectorAll('.chart-ticks span'), s => s.textContent)).toEqual(['1 月', '', '']);
    expect(screen.getByLabelText('只有你常说').textContent).toBe('好嘞20');
    expect(screen.queryByLabelText('只有 TA 常说')).toBeNull();
  });

  it('picks a friend from those talked to most, then everyone else, read once it is opened', async () => {
    const urls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      urls.push(url);
      return json({ success: true, data: url.startsWith('/analytics/rankings')
        ? [{ username: 'wxid_b', displayName: '陈屿', messageCount: 9, sentCount: 4, receivedCount: 5, lastMessageTime: 0 }]
        : [{ username: 'wxid_a', displayName: '林沐', wechatId: 'linmu' }, { username: 'wxid_b', displayName: '陈屿' }] });
    }));
    const onSelect = vi.fn();
    render(<FriendPicker selected={null} onSelect={onSelect} />);
    expect(urls).toEqual([]);
    fireEvent.click(screen.getByRole('button', { name: /选择一位好友/ }));
    await act(async () => {});
    expect(urls).toEqual(['/analytics/rankings?limit=20', '/analytics/exclude-candidates']);
    expect(screen.getAllByRole('option').map(o => o.textContent)).toEqual(['陈陈屿', '林林沐']);
    fireEvent.change(screen.getByRole('searchbox', { name: '搜索好友' }), { target: { value: 'LINMU' } });
    fireEvent.click(screen.getByRole('option'));
    expect(onSelect).toHaveBeenCalledWith({ username: 'wxid_a', displayName: '林沐', wechatId: 'linmu' });
  });

  it('reads the dual report of the year with the friend picked', async () => {
    const urls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      urls.push((init?.method ?? 'GET') + ' ' + url + (init?.body ? ' ' + init.body : ''));
      return json({ success: true, data: url.startsWith('/report/annual/years') ? { years: [2025] } : REPORT });
    }));
    const { rerender } = render(<ReportsPage mode="dual" year={null} onYear={() => {}} friend={null} onFriend={() => {}} />);
    await act(async () => {});
    expect(screen.getByText('选一位好友，生成你们两个人的报告。')).toBeTruthy();
    rerender(<ReportsPage mode="dual" year={null} onYear={() => {}} friend={{ username: 'wxid_a', displayName: '林沐' }} onFriend={() => {}} />);
    await act(async () => {});
    expect(urls).toEqual(['GET /report/annual/years', 'POST /report/dual/generate {"friend":"wxid_a","year":2025}']);
    expect(screen.getByRole('article').getAttribute('aria-label')).toBe('我自己和林沐的 2025 年双人报告');
  });
});
