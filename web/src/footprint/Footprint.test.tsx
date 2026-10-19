import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { presetRanges, timeline, type Footprint } from '../api/footprint';
import type { Contact } from '../api/contacts';
import { FootprintAi, FootprintExport, FootprintPage, FootprintView, mentionText } from './Footprint';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const t = (d: number, h: number, m = 0) => new Date(2026, 9, d, h, m).getTime() / 1000;
const DATA: Footprint = {
  summary: { private_inbound_people: 4, private_replied_people: 3, private_outbound_people: 5, private_reply_rate: 0.75, mention_count: 2, mention_group_count: 1 },
  private_sessions: [{ session_id: 'wxid_a', incoming_count: 3, outgoing_count: 2, replied: true }],
  private_segments: [
    { session_id: 'wxid_a', segment_index: 1, start_ts: t(20, 9), end_ts: t(20, 9, 30), incoming_count: 3, outgoing_count: 2, message_count: 5, replied: true, anchor_local_id: 11, anchor_create_time: t(20, 9) },
    { session_id: 'wxid_b', segment_index: 1, start_ts: t(19, 22), end_ts: t(19, 22), incoming_count: 1, outgoing_count: 0, message_count: 1, replied: false, anchor_local_id: 7, anchor_create_time: t(19, 22) },
  ],
  mentions: [{ session_id: 'room@chatroom', local_id: 99, create_time: t(20, 8), sender_username: 'wxid_c', message_content: 'wxid_c:\n@我 周六几点集合？' }],
  mention_groups: [{ session_id: 'room@chatroom', count: 2, latest_ts: t(20, 8) }],
  diagnostics: { truncated: false },
};
const CONTACTS: Record<string, Contact> = {
  wxid_a: { username: 'wxid_a', displayName: '林沐', type: 'friend' },
  wxid_b: { username: 'wxid_b', displayName: '陈屿', type: 'friend' },
  wxid_c: { username: 'wxid_c', displayName: '阿杰', type: 'friend' },
  'room@chatroom': { username: 'room@chatroom', displayName: '周末徒步群', type: 'group' },
};
const names = (u: string) => CONTACTS[u];

describe('my footprint', () => {
  it('offers today, yesterday, this week and last week, weeks from Monday', () => {
    expect(presetRanges(new Date(2026, 9, 21, 15))).toEqual([
      { label: '今天', from: '2026-10-21', to: '2026-10-21' },
      { label: '昨天', from: '2026-10-20', to: '2026-10-20' },
      { label: '本周', from: '2026-10-19', to: '2026-10-21' },
      { label: '上周', from: '2026-10-12', to: '2026-10-18' },
    ]);
    // a Sunday is the end of its week
    expect(presetRanges(new Date(2026, 9, 25, 15))[2]).toEqual({ label: '本周', from: '2026-10-19', to: '2026-10-25' });
  });

  it('puts the chats and the mentions in the order they happened, and takes the sender line off a group message', () => {
    expect(timeline(DATA).map(e => e.at)).toEqual([t(19, 22), t(20, 8), t(20, 9)]);
    expect(mentionText('wxid_c:\n@我 周六几点集合？')).toBe('@我 周六几点集合？');
    expect(mentionText('@我 没有前缀')).toBe('@我 没有前缀');
  });

  it('shows the numbers and the timeline by day, narrows it, and opens a message', () => {
    const onOpen = vi.fn();
    const onKeyword = vi.fn();
    const { rerender } = render(<FootprintView data={DATA} keyword="" onKeyword={onKeyword} names={names} onOpen={onOpen} />);
    expect(screen.getByLabelText('总览').textContent).toContain('我回复了3 人 · 75%');
    expect(screen.getByLabelText('@我的群').querySelector('ul')?.textContent).toBe('周末徒步群2');
    const items = Array.from(document.querySelectorAll('.footprint-timeline > li'), li => li.textContent);
    expect(items).toHaveLength(5);
    expect(items[1]).toContain('陈屿');
    expect(items[1]).toContain('没有回复');
    expect(items[3]).toContain('阿杰：@我 周六几点集合？');
    expect(items[4]).toContain('收到 3 · 发出 2 · 30 分钟 · 已回复');
    fireEvent.click(screen.getByRole('button', { name: '打开周末徒步群里的这条消息' }));
    expect(onOpen).toHaveBeenCalledWith('room@chatroom', 99, t(20, 8));
    fireEvent.change(screen.getByRole('searchbox', { name: '筛选时间线' }), { target: { value: '集合' } });
    expect(onKeyword).toHaveBeenCalledWith('集合');
    rerender(<FootprintView data={DATA} keyword="集合" onKeyword={onKeyword} names={names} onOpen={onOpen} />);
    expect(document.querySelectorAll('.footprint-event')).toHaveLength(1);
    rerender(<FootprintView data={DATA} keyword="没有的人" onKeyword={onKeyword} names={names} onOpen={onOpen} />);
    expect(screen.getByText('没有符合的联络。')).toBeTruthy();
  });

  it('reads the range picked, and a range of its own from the calendar', async () => {
    const urls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => { urls.push(url); return json({ success: true, data: url.startsWith('/api/chat/contacts') ? [] : DATA }); }));
    const onRange = vi.fn();
    const onData = vi.fn();
    render(<FootprintPage range={{ label: '本周', from: '2026-10-19', to: '2026-10-21' }} onRange={onRange} keyword="" onKeyword={() => {}} onOpen={() => {}} onData={onData} />);
    await act(async () => {});
    expect(urls).toContain('/insight/footprint?start=2026-10-19&end=2026-10-21');
    expect(onData).toHaveBeenLastCalledWith(DATA);
    fireEvent.click(screen.getByRole('button', { name: '自定义' }));
    expect(onRange).toHaveBeenCalledWith({ label: '2026-10-19 至 2026-10-21', from: '2026-10-19', to: '2026-10-21' });
  });

  it('asks the AI with names and counts only, and offers to turn the summary on when it is off', async () => {
    const bodies: { url: string; body: unknown }[] = [];
    let off = true;
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      if (url.startsWith('/api/chat/contacts')) return json({ success: true, data: Object.values(CONTACTS) });
      bodies.push({ url, body: JSON.parse(String(init?.body)) });
      if (url.startsWith('/config/set')) { off = false; return json({ success: true, data: {} }); }
      return off
        ? json({ success: false, error: { code: 'runtime_error', message: '请先在设置中开启「AI 足迹总结」' } }, 500)
        : json({ success: true, data: { success: true, insight: '这周和林沐聊得最多。' } });
    }));
    render(<FootprintAi range={{ label: '本周', from: '2026-10-19', to: '2026-10-21' }} data={DATA} aiReady />);
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '总结「本周」' }));
    await act(async () => {});
    expect(screen.getByRole('alert').textContent).toBe('请先在设置中开启「AI 足迹总结」');
    fireEvent.click(screen.getByRole('button', { name: '开启 AI 足迹总结' }));
    await act(async () => {});
    expect(bodies[1]).toEqual({ url: '/config/set', body: { key: 'aiFootprintEnabled', value: 'true' } });
    fireEvent.click(screen.getByRole('button', { name: '总结「本周」' }));
    await act(async () => {});
    const sent = JSON.parse((bodies[2].body as Record<string, string>)['payload-json']);
    expect(sent.rangeLabel).toBe('本周');
    expect(sent.privateSegments[0]).toEqual({ session_id: 'wxid_a', displayName: '林沐', incoming_count: 3, outgoing_count: 2, message_count: 5, replied: true });
    expect(sent.mentionGroups).toEqual([{ session_id: 'room@chatroom', displayName: '周末徒步群', count: 2 }]);
    expect(JSON.stringify(sent)).not.toContain('集合');
    expect(screen.getByLabelText('AI 足迹总结').textContent).toBe('本周这周和林沐聊得最多。');
  });

  it('exports the range as JSON, kept by the server', async () => {
    const calls: { url: string; body: unknown }[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      calls.push({ url, body: JSON.parse(String(init?.body)) });
      return json({ success: true, data: { id: 'j1', status: 'queued', command: ['weflow'], createdAt: 0, startedAt: null, finishedAt: null, exitCode: null, httpStatus: null, progress: null } }, 202);
    }));
    render(<FootprintExport range={{ label: '上周', from: '2026-10-12', to: '2026-10-18' }} />);
    fireEvent.click(screen.getByRole('button', { name: '导出「上周」为 JSON' }));
    await act(async () => {});
    expect(calls).toEqual([{ url: '/export/footprint?return=file', body: { format: 'json', start: '2026-10-12', end: '2026-10-18' } }]);
  });
});
