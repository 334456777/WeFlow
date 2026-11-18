import { afterEach, expect, it, vi } from '#test';
import { emitLive } from '../api/live';
import { getMembers } from '../api/messages';
import { loadMembers, memberName } from './LiveMessages';

afterEach(() => { vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

/** Groups whose one member is 林沐, `state.nick` (小林 unless said) in the group; while `state.down`, the server is busy. */
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
  // a failure is not kept: asked again, they are read again
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

  // a member renamed themselves in the group; messages of other conversations leave the members kept
  state.nick = '林林';
  emitLive({ type: 'message.new', sessionId: 'other@chatroom' });
  emitLive({ type: 'message.new', sessionId: 'wxid_a' });
  await getMembers('g@chatroom');
  expect(memberName('g@chatroom', 'wxid_m')).toBe('小林');
  expect(calls).toHaveLength(1);

  // a message of the group: the next to ask reads them again, both asking at once share the read
  emitLive({ type: 'message.new', sessionId: 'g@chatroom' });
  expect(memberName('g@chatroom', 'wxid_m')).toBeUndefined();
  const [names, panel] = await Promise.all([loadMembers('g@chatroom'), getMembers('g@chatroom')]);
  expect(calls).toHaveLength(2);
  expect(names.get('wxid_m')?.name).toBe('林林');
  expect(panel[0].groupNickname).toBe('林林');
  expect(memberName('g@chatroom', 'wxid_m')).toBe('林林');

  // back after a break: every group's
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
