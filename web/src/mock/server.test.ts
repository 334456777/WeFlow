import { afterEach, describe, expect, it, vi } from '#test';
import type { Job, ProfileSummary } from '../api/contract.gen';
import type { Params, SampleAnswer } from '../api/client';
import { savedKey } from '../api/setup';
import { server } from './server';

// The sample server keeps what changes for as long as the page (here, the test run) is open, and other test files use
// it too: each test reads what it changed, not what the sample starts with.
afterEach(() => { vi.useRealTimers(); });

/** The data of an answer, or the answer itself when it has none. */
const data = <T>(answer: SampleAnswer): T => {
  if (!('data' in answer)) throw new Error('no data: ' + JSON.stringify(answer));
  return answer.data as T;
};
const ask = <T>(method: 'GET' | 'POST' | 'PUT' | 'DELETE', path: string, params: Params = {}) => data<T>(server.answer(method, path, params, false));
/** Starts a job, as `Prefer: respond-async` asks. */
const start = (path: string, params: Params) => data<Job>(server.answer('POST', path, params, true));
const at = (ms: number) => vi.setSystemTime(ms);

describe('the sample server (#896)', () => {
  it('keeps settings in the page: the startup options, a new profile set up step by step, the account cleared', () => {
    ask('PUT', 'config/set', { key: 'serve_max_media', value: '3' });
    expect(ask('GET', 'api/config/get', { key: 'serve_max_media' })).toBe('3');
    ask('DELETE', 'config/unset', { key: 'serve_max_media' });
    expect(ask('GET', 'api/config/get', { key: 'serve_max_media' })).toBeNull();

    // a profile added starts empty: the database does not open until it is set up
    ask('PUT', 'config/set', { key: 'current_profile', value: 'work' });
    const added = ask<ProfileSummary[]>('GET', 'api/config/profiles').find(p => p.current)!;
    expect(added).toMatchObject({ name: 'work', complete: false, decryptKey: false });
    expect(server.answer('GET', 'api/db/test', {}, false)).toEqual({ failed: { status: 400, code: 'config_error', message: '当前档案没有设置 db_path 或 wxid' } });
    const [dir] = ask<{ candidates: { path: string; exists: boolean }[] }>('GET', 'api/db/detect').candidates.filter(c => c.exists);
    const [account] = ask<{ accounts: { wxid: string }[] }>('GET', 'api/db/wxid', { root: dir.path }).accounts;
    ask('PUT', 'config/set', { key: 'db_path', value: dir.path });
    ask('PUT', 'config/set', { key: 'wxid', value: account.wxid });
    ask('POST', 'key/image', { save: true });
    expect(server.answer('GET', 'api/db/test', {}, false)).toMatchObject({ failed: { message: '当前档案没有设置 decrypt_key' } });
    ask('PUT', 'config/set', { key: 'decrypt_key', value: 'ab'.repeat(32) });
    expect(ask('GET', 'api/db/test')).toHaveProperty('accountDir');
    expect(ask<ProfileSummary[]>('GET', 'api/config/profiles').find(p => p.current)).toMatchObject({ complete: true, imageKeys: true });

    // clearing the account takes it out of the profile
    expect(ask<{ profileKeys: string[] }>('DELETE', 'cache/clear-account', { 'dry-run': true }).profileKeys).toEqual(['db_path', 'wxid', 'decrypt_key', 'image_xor_key', 'image_aes_key']);
    expect(ask('DELETE', 'cache/clear-account', { yes: true })).toMatchObject({ profileReset: true });
    expect(ask<ProfileSummary[]>('GET', 'api/config/profiles').find(p => p.current)).toMatchObject({ complete: false, dbPath: null });
    ask('PUT', 'config/set', { key: 'current_profile', value: 'default' });
    expect(ask<ProfileSummary[]>('GET', 'api/config/profiles').map(p => [p.name, p.current, p.complete])).toEqual([['default', true, true], ['work', false, false]]);
  });

  it('leaves the people out of the statistics, and writes, reads and clears insights', () => {
    const before = ask<{ username: string }[]>('GET', 'api/analytics/rankings', { limit: 50 });
    expect(ask('PUT', 'analytics/excluded', { set: ['wxid_sample_xiaolu'] })).toEqual(['wxid_sample_xiaolu']);
    expect(ask('GET', 'api/analytics/excluded')).toEqual(['wxid_sample_xiaolu']);
    expect(ask<{ username: string }[]>('GET', 'api/analytics/rankings', { limit: 50 }).map(r => r.username)).toEqual(before.map(r => r.username).filter(u => u !== 'wxid_sample_xiaolu'));
    expect(ask('PUT', 'analytics/excluded', { set: '' })).toEqual([]);

    const unread = () => ask<{ unreadCount: number; total: number }>('GET', 'insight/records');
    const { unreadCount, total } = unread();
    expect(ask('PUT', 'insight/mark-read', { id: 'sample-insight-9' })).toEqual({ id: 'sample-insight-9' });
    expect(server.answer('PUT', 'insight/mark-read', { id: 'none' }, false)).toMatchObject({ failed: { status: 404, code: 'not_found' } });
    expect(unread()).toMatchObject({ unreadCount: unreadCount - 1, total });
    const made = ask<{ record: { sessionId: string; insight: string; read: boolean } }>('POST', 'insight/trigger', { 'session-id': 'wxid_sample_ajie' });
    expect(made.record).toMatchObject({ sessionId: 'wxid_sample_ajie', read: false });
    expect(made.record.insight).toContain('阿杰');
    expect(unread()).toMatchObject({ total: total + 1 });
    expect(ask('DELETE', 'insight/clear', { session: 'wxid_sample_ajie' })).toEqual({ removed: 2 });
  });

  it('ranks every chat and every member for a limit of 0, as the commands do (#1047)', () => {
    const chats = ask<{ username: string }[]>('GET', 'api/analytics/rankings', { limit: 50 });
    expect(ask<unknown[]>('GET', 'api/analytics/rankings', { limit: 1 })).toHaveLength(1);
    expect(ask<unknown[]>('GET', 'api/analytics/rankings', { limit: 0 })).toHaveLength(chats.length);

    const [group] = ask<{ username: string }[]>('GET', 'api/group/list');
    const members = ask<unknown[]>('GET', 'api/group/ranking', { 'chatroom-id': group.username, limit: 50 });
    expect(members.length).toBeGreaterThan(1);
    expect(ask<unknown[]>('GET', 'api/group/ranking', { 'chatroom-id': group.username, limit: 1 })).toHaveLength(1);
    expect(ask<unknown[]>('GET', 'api/group/ranking', { 'chatroom-id': group.username, limit: 0 })).toHaveLength(members.length);
  });

  it('runs jobs one after another, hands back a note for the file asked for, and cancels or forgets them', async () => {
    // after the jobs other tests started, which have ended by then
    vi.useFakeTimers({ toFake: ['Date'], now: Date.now() + 3_600_000 });
    const t0 = Date.now();
    const listed = (id: string) => ask<Job[]>('GET', 'jobs').find(j => j.id === id)!;
    const first = start('export/messages?return=file', { 'session-id': 'wxid_sample_linmu', format: ['html', 'txt'], start: '2026-11-01' });
    const second = start('export/contacts?return=file', { format: 'excel' });
    expect(first).toMatchObject({ status: 'running', file: null });
    expect(first.command).toEqual(['weflow', '--json', '--progress', 'export', 'messages', '--session-id', 'wxid_sample_linmu', '--format', 'html', '--format', 'txt', '--start', '2026-11-01']);
    expect(second.status).toBe('queued');

    at(t0 + 800);
    expect(listed(first.id).progress).toMatchObject({ message: '正在导出', percent: expect.any(Number), formats: [{ name: 'html' }, { name: 'txt' }] });

    // the second starts once the first has ended, which says what was asked for and that nothing of the sample is exported
    at(t0 + 10_000);
    const done = listed(first.id);
    const next = listed(second.id);
    expect(done).toMatchObject({ status: 'succeeded', exitCode: 0, file: { name: 'WeFlow 示例数据说明.txt' } });
    expect(next.status).toBe('succeeded');
    expect(next.startedAt).toBe(done.finishedAt);
    const note = await (await fetch(server.media('jobs/' + done.id + '/file', {}))).text();
    expect(note).toBe('这是 WeFlow 网页的示例数据，连接 weflow serve 后才能导出真实记录。\n\n任务：导出聊天记录 · 林沐\n格式：html、txt\n时间范围：2026-11-01 至 最新\n');
    expect(ask<Job>('GET', 'jobs/' + done.id).result).toEqual({ success: true, data: { out: 'WeFlow 示例数据说明.txt' } });

    // a job without `return=file` hands back nothing; one not ended is cancelled, one ended forgotten
    const key = start('key/db', { save: true });
    expect(ask<Job>('DELETE', 'jobs/' + key.id).status).toBe('cancelled');
    ask('DELETE', 'jobs/' + done.id);
    expect(server.answer('GET', 'jobs/' + done.id, {}, false)).toMatchObject({ failed: { status: 404 } });
    expect(ask<Job[]>('GET', 'jobs').filter(j => [first.id, second.id, key.id].includes(j.id)).map(j => j.status)).toEqual(['succeeded', 'cancelled']);
  });

  it('saves the database key once its job has succeeded, as a job the page knows by its command (#1138)', () => {
    vi.useFakeTimers({ toFake: ['Date'], now: Date.now() + 7_200_000 });
    const t0 = Date.now();
    ask('PUT', 'config/set', { key: 'current_profile', value: 'keyed-by-job' });
    const keyed = () => ask<ProfileSummary[]>('GET', 'api/config/profiles').find(p => p.current)!.decryptKey;
    expect(keyed()).toBe(false);
    const job = start('key/db', { save: true });
    expect(savedKey(job)).toBe('db');
    at(t0 + 1000);
    expect(ask<Job>('GET', 'jobs/' + job.id).status).toBe('running');
    expect(keyed()).toBe(false);
    at(t0 + 10_000);
    expect(ask<Job>('GET', 'jobs/' + job.id)).toMatchObject({ status: 'succeeded', result: { success: true, data: { saved: ['decrypt_key'] } } });
    expect(keyed()).toBe(true);
    ask('PUT', 'config/set', { key: 'current_profile', value: 'default' });
  });

  it('refuses the page\'s versions, which need a server', () => {
    expect(server.answer('GET', 'web/list', {}, false)).toEqual({ refused: true });
  });
});
