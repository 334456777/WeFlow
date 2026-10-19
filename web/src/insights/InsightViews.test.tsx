import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { InsightPage } from '../api/insights';
import { InsightClear, InsightContacts, InsightList, InsightTrigger } from './InsightViews';
import type { InsightsState } from './useInsights';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const PAGE: InsightPage = {
  records: [
    { id: 'r2', createdAt: Date.now() - 60_000, sessionId: 'wxid_a', displayName: '林沐', triggerReason: 'silence', insight: '好久没和林沐聊了，问问她的旅行？', read: false },
    { id: 'r1', createdAt: Date.now() - 86_400_000 * 3, sessionId: 'wxid_b', displayName: '陈屿', triggerReason: 'activity', insight: '陈屿最近常提到跑步。', read: true },
  ],
  total: 2, todayCount: 1, unreadCount: 1,
  contacts: [{ sessionId: 'wxid_a', displayName: '林沐', count: 1 }, { sessionId: 'wxid_b', displayName: '陈屿', count: 1 }],
};
const READY: InsightsState = { status: 'ready', page: PAGE, more: false, loadingMore: false };

describe('insights', () => {
  it('lists everyone with insights, all first, and searches on Enter', () => {
    const onSelect = vi.fn();
    const onKeyword = vi.fn();
    render(<InsightContacts state={READY} selected={null} onSelect={onSelect} keyword="" onKeyword={onKeyword} onRetry={() => {}} />);
    expect(Array.from(document.querySelectorAll('.conversation-name'), n => n.textContent)).toEqual(['全部见解', '林沐', '陈屿']);
    fireEvent.click(screen.getByRole('button', { name: '只看林沐，1 条' }));
    expect(onSelect).toHaveBeenCalledWith('wxid_a');
    const field = screen.getByRole('searchbox', { name: '搜索见解' });
    fireEvent.change(field, { target: { value: ' 旅行 ' } });
    expect(onKeyword).not.toHaveBeenCalled();
    fireEvent.submit(field.closest('form')!);
    expect(onKeyword).toHaveBeenCalledWith('旅行');
  });

  it('shows each insight, marks an unread one read when it is opened, and reads how it was written when asked', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => json({ success: true, data: { success: true, record: { log: {
      endpoint: '', model: 'm1', maxTokens: 0, triggerReason: 'silence', contextCount: 12, systemPrompt: '', userPrompt: '最近的聊天…', rawOutput: '好久没…', durationMs: 2300,
    } } } })));
    const onRead = vi.fn();
    const onChat = vi.fn();
    render(<InsightList state={READY} narrowed={false} canChat={s => s === 'wxid_a'} onRead={onRead} onChat={onChat} onMore={() => {}} />);
    const cards = screen.getAllByRole('article');
    expect(cards.map(c => c.getAttribute('aria-label'))).toEqual(['林沐的见解', '陈屿的见解']);
    expect(cards[0].textContent).toContain('久未联系');
    expect(cards[0].querySelector('[aria-label="未读"]')).toBeTruthy();
    fireEvent.click(cards[0]);
    expect(onRead).toHaveBeenCalledWith('r2');
    fireEvent.click(cards[1]);
    expect(onRead).toHaveBeenCalledTimes(1);
    const chat = (card: HTMLElement) => Array.from(card.querySelectorAll('button')).find(b => b.textContent === '查看聊天')!;
    expect(chat(cards[1]).disabled).toBe(true);
    fireEvent.click(chat(cards[0]));
    expect(onChat).toHaveBeenCalledWith('wxid_a');

    fireEvent.click(cards[0].querySelector('[aria-expanded]')!);
    await act(async () => {});
    expect(cards[0].querySelector('.insight-log')?.textContent).toContain('m1');
    expect(cards[0].querySelector('.insight-log')?.textContent).toContain('2.3 秒');
    expect(screen.getByText('共 2 条')).toBeTruthy();
  });

  it('writes an insight for the conversation chosen, and says why when it cannot', async () => {
    const calls: unknown[] = [];
    let fail = false;
    vi.stubGlobal('fetch', vi.fn(async (_url: string, init?: RequestInit) => {
      calls.push(JSON.parse(String(init?.body)));
      return fail
        ? json({ success: false, error: { code: 'runtime_error', message: '请先填写 API 地址和 Key' } }, 500)
        : json({ success: true, data: { success: true, record: { ...PAGE.records[0], id: 'r3' } } });
    }));
    const onDone = vi.fn();
    render(<InsightTrigger session="wxid_a" name="林沐" onDone={onDone} />);
    fireEvent.click(screen.getByRole('button', { name: '生成见解' }));
    await act(async () => {});
    expect(calls).toEqual([{ 'session-id': 'wxid_a' }]);
    expect(screen.getByText('已为「林沐」写下一条见解。')).toBeTruthy();
    expect(onDone).toHaveBeenCalled();
    fail = true;
    fireEvent.click(screen.getByRole('button', { name: '生成见解' }));
    await act(async () => {});
    expect(screen.getByRole('alert').textContent).toBe('请先填写 API 地址和 Key');
  });

  it('clears the insights of a conversation only after asking', async () => {
    const urls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => { urls.push(url); return json({ success: true, data: { success: true, removed: 1 } }); }));
    const onDone = vi.fn();
    render(<InsightClear session="wxid_a" name="林沐" count={1} onDone={onDone} />);
    fireEvent.click(screen.getByRole('button', { name: '删除「林沐」的 1 条见解' }));
    expect(urls).toEqual([]);
    fireEvent.click(screen.getByRole('button', { name: '确认删除' }));
    await act(async () => {});
    expect(urls).toEqual(['/insight/clear?session=wxid_a']);
    expect(onDone).toHaveBeenCalled();
    expect(screen.getByText('已删除 1 条。')).toBeTruthy();
  });
});
