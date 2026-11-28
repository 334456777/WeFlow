import { afterEach, expect, it, vi } from '#test';
import { emitLive } from '../api/live';
import { getMembers } from '../api/messages';
import { loadMembers, memberName } from './LiveMessages';

afterEach(() => { vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

/** 模拟只有一个成员林沐的群，他的群昵称是 `state.nick`（不给时为「小林」）；`state.down` 时服务端回答 busy。 */
function serveMembers(state: { down: boolean; nick?: string }) {
  const calls: string[] = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    calls.push(url);
    if (state.down) return json({ success: false, error: { code: 'busy', message: 'busy' } }, 429);
    return json({ success: true, data: { members: [{ username: 'wxid_m', displayName: '林沐', groupNickname: state.nick ?? '小林' }] } });
  }));
  return calls;
}

it('reads the members of a group once for the names of its messages and the members panel', async () => {
  const state = { down: true };
  const calls = serveMembers(state);
  // 失败的结果不保留：再次请求时重新读取
  await expect(getMembers('g@chatroom')).rejects.toThrow('busy');
  state.down = false;
  const [names, panel] = await Promise.all([loadMembers('g@chatroom', new AbortController().signal), getMembers('g@chatroom'), loadMembers('g@chatroom')]);
  expect(calls).toHaveLength(2);
  expect(names.get('wxid_m')?.name).toBe('小林');
  expect(panel.map(m => m.displayName)).toEqual(['林沐']);
  expect(memberName('g@chatroom', 'wxid_m')).toBe('小林');
  await getMembers('g@chatroom');
  expect(calls).toHaveLength(2);
});

it('lets a caller that left go, while the read goes on for the others', async () => {
  const calls = serveMembers({ down: false });
  const leaving = new AbortController();
  const left = loadMembers('h@chatroom', leaving.signal);
  const staying = getMembers('h@chatroom');
  leaving.abort();
  expect((await Promise.allSettled([left]))[0].status).toBe('rejected');
  expect(await staying).toHaveLength(1);
  expect((await loadMembers('h@chatroom')).get('wxid_m')?.name).toBe('小林');
  expect(calls).toHaveLength(1);
});

it('reads the members again once the group had a live message or the stream was back, not before (#201)', async () => {
  const state = { down: false, nick: '小林' };
  const calls = serveMembers(state);
  await loadMembers('g@chatroom');
  await getMembers('g@chatroom');
  expect(calls).toHaveLength(1);

  // 成员改了群昵称；其他会话的消息不影响缓存的成员
  state.nick = '林林';
  emitLive({ type: 'message.new', sessionId: 'other@chatroom' });
  emitLive({ type: 'message.new', sessionId: 'wxid_a' });
  await getMembers('g@chatroom');
  expect(memberName('g@chatroom', 'wxid_m')).toBe('小林');
  expect(calls).toHaveLength(1);

  // 这个群来了消息：下一次请求重新读取，同时请求的两方共用一次读取
  emitLive({ type: 'message.new', sessionId: 'g@chatroom' });
  expect(memberName('g@chatroom', 'wxid_m')).toBeUndefined();
  const [names, panel] = await Promise.all([loadMembers('g@chatroom'), getMembers('g@chatroom')]);
  expect(calls).toHaveLength(2);
  expect(names.get('wxid_m')?.name).toBe('林林');
  expect(panel[0].groupNickname).toBe('林林');
  expect(memberName('g@chatroom', 'wxid_m')).toBe('林林');

  // 中断后恢复：所有群的成员都要重新读取
  emitLive({ type: 'resumed' });
  expect(memberName('g@chatroom', 'wxid_m')).toBeUndefined();
  await loadMembers('g@chatroom');
  expect(calls).toHaveLength(3);
});

it('gives members read before a live message of the group to their caller, without keeping them', async () => {
  const calls = serveMembers({ down: false });
  const reading = loadMembers('g@chatroom');
  emitLive({ type: 'message.new', sessionId: 'g@chatroom' });
  expect((await reading).get('wxid_m')?.name).toBe('小林');
  expect(memberName('g@chatroom', 'wxid_m')).toBeUndefined();
  await loadMembers('g@chatroom');
  expect(calls).toHaveLength(2);
  expect(memberName('g@chatroom', 'wxid_m')).toBe('小林');
});
