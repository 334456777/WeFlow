import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import type { Profile } from '../api/setup';
import { useJobs } from '../jobs/useJobs';
import { SetupWizard } from './SetupWizard';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  sessionStorage.clear();
});

const opened: FakeEventSource[] = [];
class FakeEventSource {
  onerror: ((event: Event) => void) | null = null;
  closed = false;
  private listeners = new Map<string, ((event: MessageEvent) => void)[]>();
  constructor(readonly url: string) { opened.push(this); }
  addEventListener(type: string, listener: (event: MessageEvent) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  close() { this.closed = true; }
  emit(type: string, data: unknown) {
    for (const listener of this.listeners.get(type) ?? []) listener(new MessageEvent(type, { data: JSON.stringify(data) }));
  }
}

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const ok = (data: unknown) => json({ success: true, data });
const currentStep = () => document.querySelector('.setup-step[aria-current="step"]')?.textContent;
const PROFILE: Profile = { name: 'default', current: true, wxid: null, dbPath: null, decryptKey: false, imageKeys: false, complete: false };

interface Call { method: string; url: string; body?: Record<string, unknown>; prefer?: string }

/** A server of the setup's routes; `routes` answers by `METHOD /path`, the rest with an empty list. */
function serve(routes: Record<string, (call: Call) => Response>) {
  const calls: Call[] = [];
  vi.stubGlobal('EventSource', FakeEventSource);
  vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const call: Call = { method: init?.method ?? 'GET', url, body: init?.body ? JSON.parse(String(init.body)) : undefined, prefer: (init?.headers as Record<string, string> | undefined)?.Prefer };
    calls.push(call);
    const route = routes[call.method + ' ' + url.split('?')[0]];
    return route ? route(call) : ok([]);
  }));
  return calls;
}

function Wizard({ profile, onChanged = () => {}, onDone = () => {} }: { profile: Profile; onChanged?(): void; onDone?(): void }) {
  const jobs = useJobs(true, 0);
  return <SetupWizard profile={profile} jobs={jobs} onChanged={onChanged} onDone={onDone} onClose={() => {}} />;
}

describe('SetupWizard', () => {
  it('saves the data directory and the account it finds there', async () => {
    const calls = serve({
      'GET /api/db/detect': () => ok({ candidates: [{ path: 'D:/xwechat_files', exists: true }, { path: 'C:/gone', exists: false }] }),
      'GET /api/db/wxid': () => ok({ accounts: [{ wxid: 'wxid_a', path: 'D:/xwechat_files/wxid_a_1f2e' }] }),
      'PUT /config/set': () => ok({}),
    });
    const onChanged = vi.fn();
    render(<Wizard profile={PROFILE} onChanged={onChanged} />);
    await act(async () => {});
    // the one directory that exists, and its one account, are chosen already
    expect(screen.getByRole('button', { name: 'D:/xwechat_files' }).getAttribute('aria-pressed')).toBe('true');
    expect(screen.queryByText('C:/gone')).toBeNull();
    expect(calls.find(c => c.url.startsWith('/api/db/wxid'))?.url).toBe('/api/db/wxid?root=D%3A%2Fxwechat_files');
    expect(screen.getByRole('button', { name: /wxid_a/ }).getAttribute('aria-pressed')).toBe('true');

    fireEvent.click(screen.getByRole('button', { name: '保存并继续' }));
    await act(async () => {});
    expect(calls.filter(c => c.method === 'PUT').map(c => c.body)).toEqual([{ key: 'db_path', value: 'D:/xwechat_files' }, { key: 'wxid', value: 'wxid_a' }]);
    expect(onChanged).toHaveBeenCalled();
    expect(currentStep()).toBe('2数据库密钥');
  });

  it('says why a folder has no account', async () => {
    serve({
      'GET /api/db/detect': () => ok({ candidates: [] }),
      'GET /api/db/wxid': () => json({ success: false, error: { code: 'runtime_error', message: 'no WeChat account directory found in E:/x' } }, 500),
    });
    render(<Wizard profile={PROFILE} />);
    await act(async () => {});
    fireEvent.change(screen.getByRole('textbox', { name: '其他文件夹' }), { target: { value: 'E:/x' } });
    fireEvent.click(screen.getByRole('button', { name: '查找账号' }));
    await act(async () => {});
    expect(screen.getByRole('alert').textContent).toContain('no WeChat account directory found in E:/x');
    expect((screen.getByRole('button', { name: '保存并继续' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('gets the database key as a job that saves it, telling what to do in WeChat', async () => {
    let jobs: unknown[] = [];
    const job = (status: string) => ({ id: 'k1', status, command: ['weflow', '--json', '--progress', 'key', 'db', '--save'], createdAt: 1, startedAt: null, finishedAt: null, exitCode: null, httpStatus: null, progress: null });
    const calls = serve({
      'POST /key/db': () => { jobs = [job('running')]; return json({ success: true, data: job('running') }, 202); },
      'GET /jobs': () => ok(jobs),
      'GET /jobs/k1': () => ok({ ...job('succeeded'), result: { success: true, data: { method: 'wx_key', saved: ['decrypt_key'] } } }),
    });
    const onChanged = vi.fn();
    render(<Wizard profile={{ ...PROFILE, dbPath: 'D:/x', wxid: 'wxid_a' }} onChanged={onChanged} />);
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '获取数据库密钥' }));
    await act(async () => {});
    const started = calls.find(c => c.url.startsWith('/key/db'))!;
    expect([started.url, started.prefer, started.body]).toEqual(['/key/db', 'respond-async', { save: true }]);

    // what the job tells, on each stream of it
    const emit = (type: string, data: unknown) => { for (const source of opened.filter(s => s.url.startsWith('/jobs/k1/events') && !s.closed)) source.emit(type, data); };
    act(() => { emit('key_phase', { type: 'key_phase', phase: 'quit_wechat', pids: [42] }); });
    const doing = () => document.querySelector('.setup-running p')?.textContent;
    expect(doing()).toContain('请先完全退出微信');
    act(() => { emit('key_phase', { type: 'key_phase', phase: 'login', pid: 43 }); });
    expect(doing()).toContain('请在微信的登录窗口点击「进入微信」');

    // the job's end, as its stream tells it
    jobs = [job('succeeded')];
    const done = { ...job('succeeded'), result: { success: true, data: { method: 'wx_key', saved: ['decrypt_key'] } } };
    await act(async () => { emit('result', done); });
    expect(screen.getByRole('status').textContent).toContain('已获取数据库密钥');
    expect(onChanged).toHaveBeenCalled();
  });

  it('takes a typed key only when it is 64 hexadecimal digits', async () => {
    const calls = serve({ 'PUT /config/set': () => ok({}) });
    render(<Wizard profile={{ ...PROFILE, dbPath: 'D:/x', wxid: 'wxid_a' }} />);
    await act(async () => {});
    const field = screen.getByLabelText('数据库密钥');
    const save = screen.getByRole('button', { name: '保存' }) as HTMLButtonElement;
    fireEvent.change(field, { target: { value: 'abc' } });
    expect(save.disabled).toBe(true);
    fireEvent.change(field, { target: { value: 'ab'.repeat(32) } });
    fireEvent.click(save);
    await act(async () => {});
    expect(calls.find(c => c.method === 'PUT')?.body).toEqual({ key: 'decrypt_key', value: 'ab'.repeat(32) });
    expect((field as HTMLInputElement).value).toBe('');
  });

  it('tests the database last, and says where to go back to when it fails', async () => {
    serve({ 'GET /api/db/test': () => json({ success: false, error: { code: 'config_error', message: 'missing decrypt_key; run config set decrypt_key' } }, 409) });
    render(<Wizard profile={{ ...PROFILE, dbPath: 'D:/x', wxid: 'wxid_a', decryptKey: true }} />);
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '4验证' }));
    await act(async () => {});
    expect(screen.getByRole('alert').textContent).toBe('还没有数据库密钥。');
    // the steps the profile has done are ticked
    expect(screen.getByRole('button', { name: '已完成数据库目录' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '回到数据库密钥' }));
    expect(currentStep()).toBe('数据库密钥');
  });
});
