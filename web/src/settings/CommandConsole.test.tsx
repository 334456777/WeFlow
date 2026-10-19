import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DOC } from '../api/console.fixture';
import { CommandConsole } from './CommandConsole';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.useRealTimers(); });

const json = (body: unknown, status = 200, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json', ...headers } });

function serve() {
  const calls: { method: string; url: string; body?: unknown; prefer?: string }[] = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    const headers = (init?.headers ?? {}) as Record<string, string>;
    calls.push({ method: init?.method ?? 'GET', url, body: init?.body ? JSON.parse(String(init.body)) : undefined, prefer: headers.Prefer });
    if (url === '/openapi.json') return json(DOC);
    // `?std=in` answers the command line itself, without the `{success, data}` document
    if (url.includes('std=in')) return json({ argv: ['weflow', '--json', 'chat', 'messages', '--session-id', 'wxid_a'], stdin: null });
    if (url.startsWith('/chat/messages')) return json({ success: true, data: [{ localId: 1 }] }, 200, { 'X-WeFlow-Exit-Code': '0' });
    if (url.startsWith('/export/contacts')) return json({ success: true, data: { id: 'j1', status: 'queued', command: ['weflow', 'export', 'contacts'], createdAt: 0, startedAt: null, finishedAt: null, exitCode: null, httpStatus: null, progress: null } }, 202);
    return json({ success: true, data: [] });
  }));
  return calls;
}

describe('CommandConsole', () => {
  it('lists the commands by tag, finds them by words, and runs one from its form', async () => {
    const calls = serve();
    render(<CommandConsole pick={null} onPicked={() => {}} />);
    await act(async () => {});
    const list = screen.getByRole('list', { name: '命令' });
    expect(Array.from(list.querySelectorAll('h4'), h => h.textContent)).toEqual(['chat', 'export', 'sns']);
    fireEvent.change(screen.getByRole('searchbox', { name: '搜索命令' }), { target: { value: '导出 通讯录' } });
    expect(Array.from(list.querySelectorAll('.command-item code'), c => c.textContent)).toEqual(['export contacts']);
    fireEvent.change(screen.getByRole('searchbox', { name: '搜索命令' }), { target: { value: '' } });

    fireEvent.click(within(list).getByRole('button', { name: /chat messages/ }));
    const form = screen.getByRole('form', { name: '运行 chat messages' });
    // what is missing, then the command line it runs
    expect(within(form).getByLabelText('等价的命令行').textContent).toBe('请填写 session-id。');
    expect((within(form).getByRole('button', { name: '运行' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(within(form).getByLabelText(/session-id/), { target: { value: 'wxid_a' } });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 300)); });
    expect(within(form).getByLabelText('等价的命令行').textContent).toBe('weflow --json chat messages --session-id wxid_a');
    expect(calls.some(c => c.url === '/chat/messages?session-id=wxid_a&std=in')).toBe(true);

    fireEvent.click(within(form).getByRole('button', { name: '运行' }));
    await act(async () => {});
    const answer = screen.getByLabelText('运行结果');
    expect(answer.textContent).toContain('HTTP 200');
    expect(answer.textContent).toContain('退出码 0');
    expect(answer.querySelector('pre')?.textContent).toContain('"localId": 1');
  });

  it('runs a command that writes a file as a job, and shows a refused one without a way to run it', async () => {
    const calls = serve();
    const onPicked = vi.fn();
    const { rerender } = render(<CommandConsole pick={null} onPicked={onPicked} />);
    await act(async () => {});
    rerender(<CommandConsole pick="export contacts" onPicked={onPicked} />);
    await act(async () => {});
    expect(onPicked).toHaveBeenCalled();
    const form = screen.getByRole('form', { name: '运行 export contacts' });
    expect((within(form).getByRole('checkbox', { name: '作为任务运行' }) as HTMLInputElement).checked).toBe(true);
    fireEvent.change(within(form).getByLabelText(/^out/), { target: { value: 'contacts.json' } });
    fireEvent.click(within(form).getByRole('button', { name: '加入任务' }));
    await act(async () => {});
    expect(calls.find(c => c.url.startsWith('/export/contacts?return=file'))).toMatchObject({ method: 'POST', prefer: 'respond-async', body: { out: 'contacts.json' } });
    expect(within(form).getByText(/已加入任务中心/)).toBeTruthy();

    fireEvent.click(screen.getAllByRole('button', { name: /sns block-delete/ })[1]);
    const refused = screen.getByRole('form', { name: '运行 sns block-delete' });
    expect(refused.textContent).toContain('只读拒绝');
    expect(within(refused).queryByRole('button', { name: '运行' })).toBeNull();
  });
});
