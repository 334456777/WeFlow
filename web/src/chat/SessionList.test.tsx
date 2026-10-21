import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from '#test';
import type { ChatSession } from '../api/chat';
import { SessionList } from './SessionList';
import type { SessionsState } from './useSessions';

afterEach(cleanup);

const session = (username: string, displayName: string, time: number, unreadCount = 0): ChatSession =>
  ({ username, displayName, summary: displayName + '的消息', unreadCount, sortTimestamp: time, lastTimestamp: time, lastMsgType: 1 });

const ready: SessionsState = {
  status: 'ready',
  sessions: [session('wxid_a', '林沐', 500, 200),session('g1@chatroom', '家人群', 400, 120), session('wxid_b', '陈屿', 300), session('g2@chatroom', '旧同学', 200)],
  statuses: { 'g1@chatroom': { isFolded: true, isMuted: false }, 'g2@chatroom': { isFolded: true, isMuted: true }, wxid_b: { isFolded: false, isMuted: true } },
};

function Harness({ state = ready, onRetry = () => {} }: { state?: SessionsState; onRetry?: () => void }) {
  const [selected, setSelected] = useState<string | null>('wxid_a');
  return <SessionList state={state} sample={false} selectedId={selected} onSelect={setSelected} onRetry={onRetry} />;
}

const names = () => Array.from(document.querySelectorAll('.conversation-name'), name => name.textContent);

describe('SessionList', () => {
  it('lists the sessions with folded groups gathered, unread counts and muted ones', () => {
    render(<Harness />);
    expect(names()).toEqual(['林沐', '折叠的群聊', '陈屿']);
    expect(screen.getByLabelText('200 条未读').textContent).toBe('99+');
    // The folded groups only show that something is unread.
    expect(screen.getByLabelText('120 条未读').className).toBe('unread-dot');
    expect(screen.getByLabelText('免打扰')).toBeTruthy();
  });

  it('opens the folded groups and comes back', () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole('button', { name: '打开折叠的群聊，共 2 个' }));
    expect(names()).toEqual(['家人群', '旧同学']);
    fireEvent.click(screen.getByRole('button', { name: '返回全部会话' }));
    expect(names()).toEqual(['林沐', '折叠的群聊', '陈屿']);
  });

  it('filters by kind and searches by name', () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole('button', { name: '私聊' }));
    expect(names()).toEqual(['林沐', '陈屿']);
    fireEvent.change(screen.getByRole('searchbox', { name: '搜索会话' }), { target: { value: '同学' } });
    expect(screen.getByText('没有名称匹配的会话。')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '全部' }));
    expect(names()).toEqual(['旧同学']);
  });

  it('moves to the next conversation with the arrow keys and opens it', () => {
    render(<Harness />);
    const first = screen.getByRole('button', { name: '选择林沐' });
    first.focus();
    fireEvent.keyDown(first, { key: 'ArrowDown' });
    expect(document.activeElement?.getAttribute('aria-label')).toBe('打开折叠的群聊，共 2 个');
    fireEvent.keyDown(document.activeElement!, { key: 'End' });
    expect(screen.getByRole('button', { name: '选择陈屿' }).getAttribute('aria-pressed')).toBe('true');
    expect(document.activeElement?.getAttribute('aria-label')).toBe('选择陈屿');
    fireEvent.keyDown(document.activeElement!, { key: 'Home' });
    expect(first.getAttribute('aria-pressed')).toBe('true');
  });

  it('shows the loading rows and an error that can be retried', () => {
    render(<Harness state={{ status: 'loading' }} />);
    expect(screen.getByLabelText('正在读取会话').getAttribute('aria-busy')).toBe('true');
    cleanup();
    const retry = vi.fn();
    render(<Harness state={{ status: 'error', message: '读取会话失败' }} onRetry={retry} />);
    expect(screen.getByRole('alert').textContent).toContain('读取会话失败');
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(retry).toHaveBeenCalledTimes(1);
  });
});
