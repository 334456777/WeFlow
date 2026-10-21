import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import type { Group } from '../api/groups';
import { GroupExport, GroupPicker, GroupStats, MemberView } from './GroupAnalytics';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const ok = (data: unknown) => json({ success: true, data });
const GROUP: Group = { username: 'room1@chatroom', displayName: '周末徒步群', memberCount: 86 };
const NO_RANGE = { from: '', to: '' };
const BOB = { username: 'wxid_bob', displayName: '阿杰' };

describe('group analytics', () => {
  it('picks a group from the list, found by name', () => {
    const groups: Group[] = [GROUP, { username: 'room2@chatroom', displayName: '家人', memberCount: 6 }];
    const onSelect = vi.fn();
    render(<GroupPicker state={{ status: 'ready', groups }} selected={GROUP} onSelect={onSelect} onRetry={() => {}} />);
    const picker = screen.getByRole('button', { name: /周末徒步群/ });
    expect(picker.textContent).toContain('86 位成员');
    fireEvent.click(picker);
    expect(screen.getAllByRole('option').map(o => o.textContent)).toEqual(['周末徒步群86 位成员', '家人6 位成员']);
    expect(screen.getByRole('option', { selected: true }).textContent).toContain('周末徒步群');
    fireEvent.change(screen.getByRole('searchbox', { name: '搜索群聊' }), { target: { value: '家' } });
    fireEvent.click(screen.getByRole('option'));
    expect(onSelect).toHaveBeenCalledWith('room2@chatroom');
  });

  it('shows a group by kind, hour and its most active members over the range, and opens one', async () => {
    const urls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      urls.push(url);
      if (url.startsWith('/group/media')) return ok({ typeCounts: [{ type: 1, name: '文本', count: 90 }, { type: 3, name: '图片', count: 10 }], total: 100 });
      if (url.startsWith('/group/hours')) return ok({ hourlyDistribution: { 20: 70, 8: 30 } });
      return ok([{ member: BOB, messageCount: 60 }, { member: { username: 'wxid_lin', displayName: '林沐' }, messageCount: 40 }]);
    }));
    const onMember = vi.fn();
    render(<GroupStats group={GROUP} range={{ from: '2026-01-01', to: '' }} onMember={onMember} />);
    await act(async () => {});
    expect(urls.sort()).toEqual([
      '/group/hours?chatroom-id=room1%40chatroom&start=2026-01-01',
      '/group/media?chatroom-id=room1%40chatroom&start=2026-01-01',
      '/group/ranking?chatroom-id=room1%40chatroom&limit=20&start=2026-01-01',
    ]);
    expect(screen.getByLabelText('各类消息的条数').textContent).toContain('文本9090%');
    expect(screen.getByRole('img', { name: '每小时的消息：最多的是20 点，70 条' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '查看阿杰的统计' }));
    expect(onMember).toHaveBeenCalledWith(BOB);
  });

  it('shows a member, their words, and their messages page by page', async () => {
    const urls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      urls.push(url);
      if (url.startsWith('/group/member-messages')) {
        const later = url.includes('cursor=50');
        return ok({ messages: [{ messageKey: later ? 'k2' : 'k1', localType: 1, createTime: later ? 1_700_000_000 : 1_760_000_000, parsedContent: later ? '早些的' : '周六出发' }], hasMore: !later, nextCursor: 50 });
      }
      return ok({
        statistics: { totalMessages: 12, textMessages: 10, imageMessages: 2, voiceMessages: 0, videoMessages: 0, emojiMessages: 0, otherMessages: 0, sentMessages: 12, receivedMessages: 0, firstMessageTime: 1_700_000_000, lastMessageTime: 1_760_000_000, activeDays: 5 },
        timeDistribution: { 9: 12 }, commonPhrases: [{ phrase: '好的', count: 4 }], commonEmojis: [{ emoji: '[呲牙]', count: 3 }],
      });
    }));
    const onBack = vi.fn();
    render(<MemberView group={GROUP} member={BOB} range={NO_RANGE} onBack={onBack} />);
    await act(async () => {});
    expect(screen.getByLabelText('成员总览').textContent).toContain('发过言的天数5 天');
    expect(screen.getByLabelText('常用语').textContent).toBe('好的4');
    expect(screen.getByLabelText('常用表情').textContent).toBe('[呲牙]3');
    const texts = () => Array.from(document.querySelectorAll('.member-messages li .clamp-3'), li => li.textContent);
    expect(texts()).toEqual(['周六出发']);
    fireEvent.click(screen.getByRole('button', { name: '显示更早的消息' }));
    await act(async () => {});
    expect(urls.at(-1)).toBe('/group/member-messages?chatroom-id=room1%40chatroom&username=wxid_bob&limit=50&cursor=50');
    expect(texts()).toEqual(['周六出发', '早些的']);
    expect(screen.queryByRole('button', { name: '显示更早的消息' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '返回「周末徒步群」的统计' }));
    expect(onBack).toHaveBeenCalled();
  });

  it('exports the members, and the messages of the member shown over the range, as jobs that keep the file', async () => {
    const calls: { url: string; body: unknown }[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
      calls.push({ url, body: JSON.parse(String(init?.body)) });
      return ok({ id: 'j' + calls.length, status: 'queued', command: ['weflow'], createdAt: 0, startedAt: null, finishedAt: null, exitCode: null, httpStatus: null, progress: null });
    }));
    const { rerender } = render(<GroupExport group={GROUP} member={null} range={NO_RANGE} />);
    expect(screen.queryByRole('button', { name: /的消息$/ })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '导出成员列表' }));
    await act(async () => {});
    rerender(<GroupExport group={GROUP} member={BOB} range={{ from: '2026-01-01', to: '2026-03-31' }} />);
    fireEvent.click(screen.getByRole('button', { name: '导出阿杰的消息' }));
    await act(async () => {});
    expect(calls).toEqual([
      { url: '/group/export-members?return=file', body: { 'chatroom-id': 'room1@chatroom' } },
      { url: '/group/export-member-messages?return=file', body: { 'chatroom-id': 'room1@chatroom', username: 'wxid_bob', start: '2026-01-01', end: '2026-03-31' } },
    ]);
    expect(screen.getByText('已加入任务中心，完成后在那里下载 Excel 文件。')).toBeTruthy();
  });
});
