import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import { currentProfile, type Profile } from '../api/setup';
import { useJobs } from '../jobs/useJobs';
import { SetupDialog } from './SetupDialog';
import { SetupWizard } from './SetupWizard';
import { useProfiles } from './useProfiles';

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
const PROFILE: Profile = { name: 'default', current: true, wxid: null, dbPath: null, decryptKey: false, imageKeys: false, aiKey: false, complete: false };

interface Call { method: string; url: string; body?: Record<string, unknown>; prefer?: string }

/** 模拟配置向导用到的服务端：`routes` 按 `METHOD /path` 回答，其余的回答空列表。 */
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

function Wizard({ profile, platform, onChanged = () => {}, onDone = () => {} }: {
  profile: Profile; platform?: 'windows' | 'macos' | 'linux'; onChanged?(): void; onDone?(): void;
}) {
  const jobs = useJobs(true, 0);
  return <SetupWizard profile={profile} jobs={jobs} platform={platform} onChanged={onChanged} onDone={onDone} onClose={() => {}} />;
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
    // 唯一存在的目录和其中唯一的账号已经选好
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
    render(<Wizard profile={{ ...PROFILE, dbPath: 'D:/x', wxid: 'wxid_a' }} />);
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '获取数据库密钥' }));
    await act(async () => {});
    const started = calls.find(c => c.url.startsWith('/key/db'))!;
    expect([started.url, started.prefer, started.body]).toEqual(['/key/db', 'respond-async', { save: true }]);

    // 向这个作业每个打开着的事件流发送事件
    const emit = (type: string, data: unknown) => { for (const source of opened.filter(s => s.url.startsWith('/jobs/k1/events') && !s.closed)) source.emit(type, data); };
    act(() => { emit('key_phase', { type: 'key_phase', phase: 'quit_wechat', pids: [42] }); });
    const doing = () => document.querySelector('.setup-running p')?.textContent;
    expect(doing()).toContain('请先完全退出微信');
    act(() => { emit('key_phase', { type: 'key_phase', phase: 'login', pid: 43 }); });
    expect(doing()).toContain('请在微信的登录窗口点击「进入微信」');

    // 进入往返缓存时，页面不保持作业的事件流（#799）；重新显示时重新打开，作业再次发来之前，仍显示用户要做的事
    const streams = () => opened.filter(s => s.url.startsWith('/jobs/k1/events') && !s.closed).length;
    expect(streams()).toBe(2);
    act(() => { window.dispatchEvent(Object.assign(new Event('pagehide'), { persisted: true })); });
    expect(streams()).toBe(0);
    await act(async () => { window.dispatchEvent(Object.assign(new Event('pageshow'), { persisted: true })); });
    expect(streams()).toBe(2);
    expect(doing()).toContain('请在微信的登录窗口点击「进入微信」');

    // 事件流告知作业结束；重新读取档案的是 useProfiles，不是这一步
    jobs = [job('succeeded')];
    const done = { ...job('succeeded'), result: { success: true, data: { method: 'wx_key', saved: ['decrypt_key'] } } };
    await act(async () => { emit('result', done); });
    expect(screen.getByRole('status').textContent).toContain('已获取数据库密钥');
    expect(screen.getByRole('button', { name: '获取数据库密钥' })).toBeTruthy();
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

  it('gets the image keys from the cache on Windows, Linux and macOS', async () => {
    const keyed = { ...PROFILE, dbPath: 'D:/x', wxid: 'wxid_a', decryptKey: true };
    const calls = serve({
      'POST /key/image': () => ok({ verified: true, saved: ['image_xor_key', 'image_aes_key'] }),
    });
    const fetched = async (platform: 'windows' | 'macos' | 'linux') => {
      cleanup();
      render(<Wizard profile={keyed} platform={platform} />);
      await act(async () => {});
      fireEvent.click(screen.getByRole('button', { name: '获取图片密钥' }));
      await act(async () => {});
      const call = calls.at(-1)!;
      return [call.url, call.body, screen.getByText(/密钥/, { selector: '.setup-note' }).textContent];
    };
    expect(await fetched('linux')).toEqual(['/key/image', { save: true }, '已获取图片密钥，并用本机的图片验证过。']);
    expect(await fetched('windows')).toEqual(['/key/image', { save: true }, '已获取图片密钥，并用本机的图片验证过。']);
    expect(await fetched('macos')).toEqual(['/key/image', { save: true }, '已获取图片密钥，并用本机的图片验证过。']);
  });

  it('tests the database last, and says where to go back to when it fails', async () => {
    serve({ 'GET /api/db/test': () => json({ success: false, error: { code: 'config_error', message: 'missing decrypt_key; run config set decrypt_key' } }, 409) });
    render(<Wizard profile={{ ...PROFILE, dbPath: 'D:/x', wxid: 'wxid_a', decryptKey: true }} />);
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '4验证' }));
    await act(async () => {});
    expect(screen.getByRole('alert').textContent).toBe('还没有数据库密钥。');
    // 档案已经完成的步骤打了勾
    expect(screen.getByRole('button', { name: '已完成数据库目录' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '回到数据库密钥' }));
    expect(currentStep()).toBe('数据库密钥');
  });
});

describe('SetupDialog', () => {
  function Dialog({ profile }: { profile: Profile }) {
    const jobs = useJobs(true, 0);
    return <SetupDialog open profile={profile} jobs={jobs} onChanged={() => {}} onClose={() => {}} onDone={() => {}} />;
  }

  /** 工作区里的对话框：作业和档案比对话框存在得久。 */
  function Workspace({ open }: { open: boolean }) {
    const jobs = useJobs(true, 0);
    const profiles = useProfiles(true, 0, jobs.jobs);
    const profile = profiles.profiles ? currentProfile(profiles.profiles) : undefined;
    return <SetupDialog open={open} profile={profile} jobs={jobs} onChanged={profiles.reload} onClose={() => {}} onDone={() => {}} />;
  }

  /** 模拟服务端：它的 `key db --save` 作业 `id` 一直运行，直到 `finish` 结束它；之后档案里就有了密钥。 */
  function keyServer(id: string) {
    const job = (status: string) => ({ id, status, command: ['weflow', '--json', '--progress', 'key', 'db', '--save'], createdAt: 1, startedAt: null, finishedAt: null, exitCode: null, httpStatus: null, progress: null });
    let jobs: unknown[] = [];
    let keyed = false;
    const calls = serve({
      'GET /api/config/profiles': () => ok([{ ...PROFILE, dbPath: 'D:/x', wxid: 'wxid_a', decryptKey: keyed }]),
      'POST /key/db': () => { jobs = [job('running')]; return json({ success: true, data: job('running') }, 202); },
      'GET /jobs': () => ok(jobs),
      ['GET /jobs/' + id]: () => ok({ ...job(keyed ? 'succeeded' : 'running'), result: keyed ? { success: true, data: { saved: ['decrypt_key'] } } : null }),
    });
    const finish = async () => {
      keyed = true;
      jobs = [job('succeeded')];
      const done = { ...job('succeeded'), result: { success: true, data: { saved: ['decrypt_key'] } } };
      await act(async () => { for (const source of opened.filter(s => s.url.startsWith('/jobs/' + id + '/events') && !s.closed)) source.emit('result', done); });
    };
    return { calls, finish, started: () => calls.filter(c => c.method === 'POST' && c.url.startsWith('/key/db')).length };
  }

  it('shows the database key job running after another step, and starts no second one (#1138)', async () => {
    const { started } = keyServer('k2');
    render(<Workspace open />);
    await act(async () => {});
    expect(currentStep()).toContain('数据库密钥');
    fireEvent.click(screen.getByRole('button', { name: '获取数据库密钥' }));
    await act(async () => {});
    expect(document.querySelector('.setup-running')).not.toBeNull();

    fireEvent.click(screen.getByRole('button', { name: '3图片密钥' }));
    fireEvent.click(screen.getByRole('button', { name: '2数据库密钥' }));
    await act(async () => {});
    expect(currentStep()).toContain('数据库密钥');
    expect(document.querySelector('.setup-running')?.textContent).toContain('正在准备');
    expect(screen.getByRole('button', { name: '取消' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: '获取数据库密钥' })).toBeNull();
    expect(started()).toBe(1);
  });

  it('reads the profile again when the job saved the key with the setup closed (#1138)', async () => {
    const { calls, finish } = keyServer('k3');
    const profileReads = () => calls.filter(c => c.url.startsWith('/api/config/profiles')).length;
    const { rerender } = render(<Workspace open />);
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '获取数据库密钥' }));
    await act(async () => {});

    rerender(<Workspace open={false} />);
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
    expect(document.querySelector('.setup-content')).toBeNull();
    const before = profileReads();
    await finish();
    expect(profileReads()).toBe(before + 1);

    rerender(<Workspace open />);
    await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: '已完成数据库密钥' }));
    expect(document.querySelector('.setup-state')?.textContent).toBe('当前档案已有数据库密钥。');
    expect((screen.getByRole('button', { name: '下一步' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('starts a profile added under a set-up one at the data directory (#898)', () => {
    serve({});
    const setUp: Profile = { ...PROFILE, wxid: 'wxid_a', dbPath: 'D:/xwechat_files', decryptKey: true };
    const { rerender } = render(<Dialog profile={setUp} />);
    expect(currentStep()).toContain('图片密钥');
    rerender(<Dialog profile={{ ...PROFILE, name: 'work' }} />);
    expect(currentStep()).toContain('数据库目录');
    expect(screen.queryByRole('button', { name: '已完成数据库目录' })).toBeNull();
  });
});
