import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { getBizAccounts } from '../api/biz';
import { emitLive } from '../api/live';
import { BizPushes } from './BizPushes';
import { sessionRows, withOfficials } from './sessions';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown) => new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
const session = { username: 'gh_news', displayName: '每日新闻', summary: '', unreadCount: 0, sortTimestamp: 0, lastTimestamp: 0, lastMsgType: 49 };
const push = (n: number, extra: object = {}) => ({
  localId: n, createTime: 1_760_000_000 - n * 3600, title: '标题 ' + n, description: '摘要 ' + n, url: 'https://mp.example/' + n, cover: '',
  contentList: [], ...extra,
});

describe('BizPushes (#203)', () => {
  it('shows pushes as article cards, newest first, and pages on', async () => {
    const calls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      calls.push(url);
      const offset = Number(new URL(url, 'http://x').searchParams.get('offset'));
      const pushes = offset === 0
        ? [push(0, { contentList: [
          { title: '标题 0', url: 'https://mp.example/0', cover: '', summary: '' },
          { title: '第二篇', url: 'https://mp.example/0b', cover: '', summary: '' },
        ] }), ...Array.from({ length: 19 }, (_, i) => push(i + 1))]
        : [push(20)];
      return json({ success: true, data: { username: 'gh_news', messages: pushes } });
    }));
    render(<BizPushes session={session} />);
    await screen.findByRole('heading', { name: '每日新闻' });
    expect(calls[0]).toContain('/biz/messages?username=gh_news&limit=20&offset=0');
    const cards = document.querySelectorAll('.biz-push');
    expect(cards).toHaveLength(20);
    // the lead links to its article; the other article of the push follows as a row, the lead not twice
    expect(screen.getByRole('link', { name: '标题 0' }).getAttribute('href')).toBe('https://mp.example/0');
    expect(screen.getByRole('link', { name: '第二篇' }).getAttribute('target')).toBe('_blank');
    expect(cards[0].querySelectorAll('.biz-item')).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: '加载更早的推送' }));
    await waitFor(() => expect(document.querySelectorAll('.biz-push')).toHaveLength(21));
    expect(calls[1]).toContain('offset=20');
    expect(screen.queryByRole('button', { name: '加载更早的推送' })).toBeNull();
  });

  it('says why there is nothing, and reads again on a live push', async () => {
    let pushes: object[] = [];
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: true, data: { messages: pushes } })));
    render(<BizPushes session={session} />);
    expect(await screen.findByText(/本机没有这个公众号的推送/)).toBeTruthy();
    pushes = [push(1)];
    await act(async () => { emitLive({ type: 'message.new', sessionId: 'gh_news' }); });
    expect(await screen.findByRole('link', { name: '标题 1' })).toBeTruthy();
  });
});

describe('official accounts in the list (#203)', () => {
  it('reads the accounts followed from their contact rows', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: true, data: { accounts: [
      { username: 'gh_a', nick_name: '甲', remark: '', big_head_url: 'https://wx.example/a.jpg', lastTime: 1_760_000_000 },
      { username: 'gh_b', alias: 'b', lastTime: null },
      { nick_name: 'no name' },
    ] } })));
    expect(await getBizAccounts()).toEqual([
      { username: 'gh_a', name: '甲', avatarUrl: 'https://wx.example/a.jpg', lastTime: 1_760_000_000 },
      { username: 'gh_b', name: 'b', avatarUrl: undefined, lastTime: 0 },
    ]);
  });

  it('adds the ones without a conversation under 「公众号」 only, and finds them by name', () => {
    const listed = [{ ...session }, { ...session, username: 'wxid_x', displayName: '林沐', sortTimestamp: 1_760_000_000 }];
    const officials = [{ ...session }, { ...session, username: 'gh_more', displayName: '城市手记' }];
    expect(withOfficials(listed, officials).map(s => s.username)).toEqual(['gh_news', 'wxid_x', 'gh_more']);
    expect(sessionRows(listed, {}, 'official', '', false, officials).map(r => r.id)).toEqual(['gh_news', 'gh_more']);
    expect(sessionRows(listed, {}, 'all', '', false, officials).map(r => r.id)).toEqual(['gh_news', 'wxid_x']);
    expect(sessionRows(listed, {}, 'official', '城市', false, officials).map(r => r.id)).toEqual(['gh_more']);
  });
});
