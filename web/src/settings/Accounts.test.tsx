import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from '#test';
import type { Profile } from '../api/setup';
import { Accounts, AccountsSkeleton } from './Accounts';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
const PROFILES: Profile[] = [
  { name: 'default', current: true, wxid: 'wxid_a', dbPath: 'D:/xwechat_files', decryptKey: true, imageKeys: true, aiKey: false, complete: true },
  { name: 'work', current: false, wxid: 'wxid_w', dbPath: null, decryptKey: false, imageKeys: false, aiKey: false, complete: false },
];

function serve() {
  const calls: { method: string; url: string; body?: unknown }[] = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    calls.push({ method: init?.method ?? 'GET', url, body: init?.body ? JSON.parse(String(init.body)) : undefined });
    if (url.startsWith('/cache/clear-account?dry-run=true')) return json({ success: true, data: { dryRun: true, paths: [{ path: 'C:/weflow/cache/Images/wxid_a', bytes: 2048 }], bytes: 2048, profileKeys: ['db_path', 'wxid', 'decrypt_key'] } });
    return json({ success: true, data: {} });
  }));
  return calls;
}

describe('Accounts', () => {
  it('lists each profile with its account and state, and switches to another', async () => {
    const calls = serve();
    const onChanged = vi.fn();
    render(<Accounts profiles={PROFILES} onChanged={onChanged} onSetup={() => {}} />);
    const rows = Array.from(document.querySelectorAll('.account'), row => row.textContent);
    expect(rows).toEqual(['default当前wxid_aD:/xwechat_files已配置配置向导', 'workwxid_w还没有配置切换']);
    fireEvent.click(screen.getByRole('button', { name: '切换' }));
    await act(async () => {});
    expect(calls).toEqual([{ method: 'PUT', url: '/config/set', body: { key: 'current_profile', value: 'work' } }]);
    expect(onChanged).toHaveBeenCalled();
  });

  it('adds a profile with a new name, then opens the setup', async () => {
    const calls = serve();
    const onSetup = vi.fn();
    render(<Accounts profiles={PROFILES} onChanged={() => {}} onSetup={onSetup} />);
    fireEvent.click(screen.getByRole('button', { name: '添加账号' }));
    const field = screen.getByRole('textbox', { name: '新档案的名称' });
    const submit = screen.getByRole('button', { name: '添加并配置' }) as HTMLButtonElement;
    for (const [typed, why] of [['work', '已经有这个档案了。'], ['a.b', '档案名称只能包含文字、数字、- 和 _。']]) {
      fireEvent.change(field, { target: { value: typed } });
      expect(submit.disabled).toBe(true);
      expect(screen.getByText(why)).toBeTruthy();
    }
    fireEvent.change(field, { target: { value: ' 家里 ' } });
    fireEvent.click(submit);
    await act(async () => {});
    expect(calls).toEqual([{ method: 'PUT', url: '/config/set', body: { key: 'current_profile', value: '家里' } }]);
    expect(onSetup).toHaveBeenCalled();
  });

  it('shows what clearing the account removes before it does, and clears only when confirmed', async () => {
    const calls = serve();
    render(<Accounts profiles={PROFILES} onChanged={() => {}} onSetup={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: '清除账号数据' }));
    await act(async () => {});
    const plan = screen.getByRole('group', { name: '将要清除的内容' });
    expect(plan.textContent).toContain('共 2.0 KB');
    expect(plan.textContent).toContain('C:/weflow/cache/Images/wxid_a');
    expect(plan.textContent).toContain('db_path、wxid、decrypt_key');
    expect(calls.map(c => c.method + ' ' + c.url)).toEqual(['DELETE /cache/clear-account?dry-run=true']);
    fireEvent.click(screen.getByRole('button', { name: '确认清除' }));
    await act(async () => {});
    expect(calls.map(c => c.method + ' ' + c.url)).toEqual(['DELETE /cache/clear-account?dry-run=true', 'DELETE /cache/clear-account?yes=true']);
    expect(screen.queryByRole('group', { name: '将要清除的内容' })).toBeNull();
  });

  it('keeps the place of the accounts while they are read', () => {
    render(<AccountsSkeleton />);
    const wait = screen.getByLabelText('正在读取账号配置');
    expect(wait.getAttribute('aria-busy')).toBe('true');
    expect(wait.querySelectorAll('.account .skeleton')).toHaveLength(8);
  });
});
