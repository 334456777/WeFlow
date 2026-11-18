import { afterEach, expect, it, vi } from '#test';
import { getMembers } from '../api/messages';
import { loadMembers, memberName } from './LiveMessages';

afterEach(() => { vi.unstubAllGlobals(); });

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

/** Groups whose one member is 林沐, 小林 in the group; while `state.down`, the server is busy. */
function serveMembers(state: { down: boolean }) {
  const calls: string[] = [];
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    calls.push(url);
    if (state.down) return json({ success: false, error: { code: 'busy', message: 'busy' } }, 429);
    return json({ success: true, data: { members: [{ username: 'wxid_m', displayName: '林沐', groupNickname: '小林' }] } });
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
