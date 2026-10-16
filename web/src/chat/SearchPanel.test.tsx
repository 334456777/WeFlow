import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ChatSession } from '../api/chat';
import type { SearchHit } from '../api/messages';
import { SearchPanel, secondsOf } from './SearchPanel';
import { useSearch } from './useSearch';

const json = (body: unknown) => new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
const at = (day: number, hour = 12) => new Date(2026, 9, day, hour).getTime() / 1000;

const sessions = [
  { username: 'wxid_a', displayName: '林沐' },
  { username: 'g@chatroom', displayName: '设计协作群' },
] as ChatSession[];

const hit = (n: number, session = 'wxid_a', extra: Partial<SearchHit> = {}): SearchHit => ({
  _cursor: 'c.' + n, _session_id: session, localId: n, localType: 1, createTime: at(10), isSend: 0,
  senderUsername: session === 'wxid_a' ? 'wxid_a' : 'wxid_c', parsedContent: '旅行计划 Trip ' + n, ...extra,
});

let calls: string[] = [];
function serve(hits: (url: URL) => SearchHit[]) {
  calls = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    calls.push(url);
    const parsed = new URL(url, 'http://x');
    if (parsed.pathname.endsWith('/group/members')) return json({ success: true, data: { members: [{ username: 'wxid_c', displayName: '陈屿', groupNickname: '小陈' }] } });
    if (parsed.pathname.endsWith('/date-counts')) return json({ success: true, data: { sessionId: 'wxid_a', counts: { '2026-10-03': 40, '2026-10-10': 2 } } });
    return json({ success: true, data: hits(parsed) });
  }));
}

function Panel({ onOpenHit = () => {}, onJumpDay = () => {} }: { onOpenHit?: (hit: SearchHit) => void; onJumpDay?: (id: string, day: string) => void }) {
  const search = useSearch();
  return <SearchPanel session={sessions[0]} sessions={sessions} search={search} onOpenHit={onOpenHit} onJumpDay={onJumpDay} />;
}

const submit = (text: string) => {
  fireEvent.change(screen.getByRole('searchbox'), { target: { value: text } });
  fireEvent.click(screen.getByRole('button', { name: '搜索' }));
};

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(2026, 9, 16, 10));
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.useRealTimers(); });

describe('SearchPanel', () => {
  it('searches this conversation, marks the keyword, pages on, and opens a hit', async () => {
    serve(url => {
      const offset = Number(url.searchParams.get('offset') || 0);
      return Array.from({ length: offset ? 3 : 20 }, (_, i) => hit(offset + i, 'wxid_a', i === 0 ? { isSend: 1 } : {}));
    });
    const opened: SearchHit[] = [];
    render(<Panel onOpenHit={h => opened.push(h)} />);
    submit(' trip ');
    await screen.findByText('已显示 20 条记录', { exact: false });
    const query = new URL(calls[0], 'http://x').searchParams;
    expect(query.get('keyword')).toBe('trip');
    expect(query.get('session-id')).toBe('wxid_a');
    expect(query.get('parsed')).toBe('true');
    expect(query.get('fields')).toContain('_session_id');
    expect(query.get('start')).toBeNull();
    const results = screen.getAllByRole('button', { name: /旅行计划/ });
    // mine, then the other party by the conversation's name; the match marked whatever its case
    expect(results[0].querySelector('strong')!.textContent).toBe('我');
    expect(results[1].querySelector('strong')!.textContent).toBe('林沐');
    expect(results[1].querySelector('mark')!.textContent).toBe('Trip');

    fireEvent.click(screen.getByRole('button', { name: '加载更多' }));
    await screen.findByText('找到 23 条记录', { exact: false });
    expect(new URL(calls[1], 'http://x').searchParams.get('offset')).toBe('20');
    fireEvent.click(results[1]);
    expect(opened[0]._cursor).toBe('c.1');
  });

  it('searches all conversations within a time range', async () => {
    serve(() => [hit(1, 'g@chatroom')]);
    render(<Panel />);
    fireEvent.click(screen.getByRole('button', { name: '全部会话' }));
    fireEvent.click(screen.getByRole('button', { name: '筛选搜索日期' }));
    fireEvent.click(screen.getByRole('button', { name: /^开始日期/ }));
    fireEvent.click(screen.getByRole('gridcell', { name: /^2026年10月3日/ }));
    fireEvent.click(screen.getByRole('gridcell', { name: /^2026年10月9日/ }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    submit('trip');
    await screen.findByText('找到 1 条记录', { exact: false });
    const query = new URL(calls.filter(c => c.includes('/chat/search')).at(-1)!, 'http://x').searchParams;
    expect(query.get('session-id')).toBeNull();
    expect(Number(query.get('start'))).toBe(secondsOf({ from: '2026-10-03', to: '' }).start);
    expect(Number(query.get('end'))).toBe(new Date(2026, 9, 9, 23, 59, 59).getTime() / 1000);
    // a hit of a group: the group and the sender, by the name in the group once its members are read
    await waitFor(() => expect(screen.getByRole('button', { name: /旅行计划/ }).querySelector('strong')!.textContent).toBe('设计协作群 · 小陈'));
  });

  it('jumps to a day picked in a calendar that marks the days with messages', async () => {
    serve(() => []);
    const jumps: [string, string][] = [];
    render(<Panel onJumpDay={(id, day) => jumps.push([id, day])} />);
    fireEvent.click(screen.getByRole('button', { name: /^跳到日期/ }));
    await waitFor(() => expect(screen.getByRole('gridcell', { name: /^2026年10月3日/ }).className).toContain('marked'));
    expect(calls.some(c => c.includes('/chat/date-counts?session-id=wxid_a'))).toBe(true);
    const busy = screen.getByRole('gridcell', { name: /^2026年10月3日/ });
    expect(busy.dataset.heat).toBe('3');
    expect(screen.getByRole('gridcell', { name: /^2026年10月10日/ }).dataset.heat).toBe('1');
    expect(screen.getByRole('gridcell', { name: /^2026年10月4日/ }).className).toContain('quiet');
    fireEvent.click(busy);
    expect(jumps).toEqual([['wxid_a', '2026-10-03']]);
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(screen.getByRole('button', { name: '跳到日期：2026-10-03' })).toBeTruthy();
  });
});

describe('secondsOf (#241)', () => {
  const zone = process.env.TZ;
  afterEach(() => { process.env.TZ = zone; });
  const local = (y: number, m: number, d: number, h = 0, min = 0, sec = 0) => new Date(y, m - 1, d, h, min, sec).getTime() / 1000;
  const midnight = (day: string) => { const [y, m, d] = day.split('-').map(Number); return local(y, m, d); };

  it('ends a day at its last second, however long the day is', () => {
    for (const tz of ['America/Los_Angeles', 'UTC', 'Asia/Tokyo']) {
      process.env.TZ = tz;
      // a 25-hour and a 23-hour day (in Los Angeles), across a month, across a year
      for (const [from, to, end] of [
        ['2026-11-01', '2026-11-01', local(2026, 11, 1, 23, 59, 59)],
        ['2026-03-08', '2026-03-08', local(2026, 3, 8, 23, 59, 59)],
        ['2026-10-30', '2026-11-02', local(2026, 11, 2, 23, 59, 59)],
        ['2026-12-31', '2027-01-01', local(2027, 1, 1, 23, 59, 59)],
      ] as const) {
        const range = secondsOf({ from, to });
        expect(range.start, tz + ' ' + from).toBe(midnight(from));
        expect(range.end, tz + ' ' + to).toBe(end);
        // the next day is left out
        expect(range.end + 1, tz + ' ' + to).toBe(midnight(to === '2027-01-01' ? '2027-01-02' : to === '2026-11-02' ? '2026-11-03' : to.slice(0, 8) + String(Number(to.slice(8)) + 1).padStart(2, '0')));
      }
    }
    process.env.TZ = 'America/Los_Angeles';
    expect(secondsOf({ from: '2026-11-01', to: '2026-11-01' }).end - midnight('2026-11-01')).toBe(25 * 3600 - 1);
    expect(secondsOf({ from: '2026-03-08', to: '2026-03-08' }).end - midnight('2026-03-08')).toBe(23 * 3600 - 1);
  });

  it('leaves an open end open', () => {
    expect(secondsOf({ from: '', to: '' })).toEqual({ start: 0, end: 0 });
    expect(secondsOf({ from: '2026-10-03', to: '' }).end).toBe(0);
  });
});
