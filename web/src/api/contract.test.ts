// The answers of the contract as weflow-cli builds them (contract/samples.json, #400), handed to the page's own
// code. A test of weflow-cli checks the same samples, and the answers a running server sends, against the contract,
// so what is read here is what the server answers, not what a fixture of the page supposes it does.
import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it, vi } from '#test';
import { dayOf, formatClock, formatFileSize, isMine, kindOf, withoutLabel } from '../chat/messages';
import { formatSessionTime, kindOf as sessionKind, sessionRows } from '../chat/sessions';
import { contactLine } from '../contacts/ContactList';
import { jobDetail } from '../jobs/TaskCenter';
import { getSessions } from './chat';
import { ApiError } from './client';
import { countKinds, getContacts, matchContacts } from './contacts';
import type * as Contract from './contract.gen';
import { listResources, type ResourceQuery } from './details';
import { describeCommand, isFinished, listJobs, type Job } from './jobs';
import { getMessages, searchMessages, type ParsedMessage } from './messages';

const TEXT = readFileSync(new URL('../../contract/samples.json', import.meta.url), 'utf8');
const SAMPLES = JSON.parse(TEXT) as {
  ParsedMessage: Contract.ParsedMessage[];
  Session: Contract.Session[];
  Contact: Contract.Contact[];
  ResourcePage: Contract.ResourcePage[];
  Error: Contract.Error[];
  Job: Contract.Job[];
};

/** What `fetch` was asked, in turn. */
let asked: URL[] = [];

/** `fetch` answers `replies` in turn, as the server sends them. */
function serve(...replies: Array<{ status: number; doc: unknown }>) {
  asked = [];
  vi.stubGlobal('fetch', async (input: string) => {
    asked.push(new URL(input, 'http://127.0.0.1:18108'));
    const reply = replies.shift();
    if (!reply) throw new Error('asked once more than expected: ' + input);
    return new Response(JSON.stringify(reply.doc), { status: reply.status, headers: { 'Content-Type': 'application/json' } });
  });
}
const ok = (data: unknown) => ({ status: 200, doc: { success: true, data } });
const failed = (status: number, error: Contract.Error) => ({ status, doc: { success: false, error } });

const zone = process.env.TZ;
afterEach(() => {
  vi.unstubAllGlobals();
  process.env.TZ = zone;
});

describe('the samples of the contract (#400)', () => {
  it('hold no number JavaScript would read otherwise; the ids that could be are strings', () => {
    const numbers = TEXT.replace(/"(?:[^"\\]|\\.)*"/g, '""').match(/-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g) ?? [];
    expect(numbers.length).toBeGreaterThan(50);
    expect(numbers.filter(n => String(Number(n)) !== n)).toEqual([]);
    expect(TEXT).toContain('"serverId": "18446744073709551615"');
  });

  it('are each kind of message the chat window shows', async () => {
    serve(ok(SAMPLES.ParsedMessage));
    const messages = await getMessages('wxid_bob', null);
    expect(messages.map(m => m.serverId)).toEqual(['9007199254740993', '18446744073709551615', '0', '0', '9007199254740995']);
    expect(messages.map(kindOf)).toEqual(['text', 'file', 'quote', 'system', 'image']);
    expect(messages.map(isMine)).toEqual([false, true, false, false, true]);
    const [, file, quote, system] = messages;
    expect([withoutLabel(file.parsedContent), formatFileSize(file.fileSize)]).toEqual(['report.pdf', '2.0 KB']);
    expect([quote.quotedContent, quote.quotedSender]).toEqual(['are you there?', 'Me']);
    // a row that says neither who sent it nor whether I did
    expect([system.isSend, system.senderUsername]).toEqual([null, null]);
    // an optional field the message has nothing for is left out, not null
    expect('fileName' in quote).toBe(false);
  });

  it('page on from a cursor as the server wrote it, two messages with the same sort_seq apart', async () => {
    // the page's messages ask for `_cursor`, which the contract's leaves optional
    const [first, , , lastOf2023, firstOf2024] = SAMPLES.ParsedMessage as Array<Contract.ParsedMessage & ParsedMessage>;
    expect(lastOf2023.sortSeq).toBe(firstOf2024.sortSeq);
    expect(lastOf2023._cursor).not.toBe(firstOf2024._cursor);
    serve(ok([]), ok([]));
    await getMessages('room1@chatroom', { before: firstOf2024._cursor });
    await getMessages('room1@chatroom', { after: first._cursor });
    expect(asked.map(url => [url.searchParams.get('session-id'), url.searchParams.get('before') ?? url.searchParams.get('after')]))
      .toEqual([['room1@chatroom', firstOf2024._cursor], ['room1@chatroom', first._cursor]]);
  });

  it('tell the day of a message by the local time of its seconds', () => {
    const [lastOf2023, firstOf2024] = SAMPLES.ParsedMessage.slice(3);
    process.env.TZ = 'UTC';
    expect([dayOf(lastOf2023.createTime), dayOf(firstOf2024.createTime)]).toEqual(['2023-12-31', '2024-01-01']);
    expect(formatClock(firstOf2024.createTime)).toBe('00:00');
    process.env.TZ = 'Asia/Shanghai';
    expect([dayOf(lastOf2023.createTime), dayOf(firstOf2024.createTime)]).toEqual(['2024-01-01', '2024-01-01']);
    expect(formatClock(lastOf2023.createTime)).toBe('07:59');
  });

  it('say which conversation a search hit is in', async () => {
    serve(ok(SAMPLES.ParsedMessage.slice(0, 1)));
    const [hit] = await searchMessages({ keyword: 'hello', sessionId: null, start: 0, end: 0 }, 0);
    expect([hit._session_id, hit.parsedContent]).toEqual(['wxid_bob', 'hello']);
  });

  it('are the rows of the conversation list', async () => {
    process.env.TZ = 'UTC';
    serve(ok(SAMPLES.Session));
    const sessions = await getSessions();
    expect(sessions.map(s => sessionKind(s.username))).toEqual(['private', 'group', 'official']);
    const rows = sessionRows(sessions, {}, 'all', '');
    expect(rows.map(r => [r.name, r.preview, r.unread])).toEqual([
      ['Bob', 'hello', 2], ['Project Room', 'Bob in room：ok', 0], ['News', '查看公众号历史消息', 0],
    ]);
    // an official account WeChat lists without a conversation has no time to show
    const now = new Date('2024-01-01T12:00:00Z');
    expect(rows.map(r => formatSessionTime(r.time, now))).toEqual(['2023/11/14', '00:00', '']);
    expect(rows[2].avatarUrl).toBeUndefined();
  });

  it('are the contacts of each kind, also with none of their optional fields', async () => {
    serve(ok(SAMPLES.Contact));
    const contacts = await getContacts();
    expect(countKinds(contacts)).toEqual({ friend: 1, group: 1, official: 0, former_friend: 1 });
    expect(matchContacts(contacts, 'BOBBY_ID').map(c => c.username)).toEqual(['wxid_bob']);
    expect(matchContacts(contacts, 'gone').map(c => c.username)).toEqual(['wxid_gone']);
    expect(contacts.map(contactLine)).toEqual(['Bob · 广东 深圳', '群聊', '曾经的好友']);
  });

  it('page through resources until a page says no more follow', async () => {
    const query: ResourceQuery = { session: null, types: [], from: '', to: '' };
    const [more, last, none] = SAMPLES.ResourcePage;
    serve(ok(more), ok(last), ok(none));
    const first = await listResources(query, null, 1);
    expect([first.hasMore, first.nextCursor]).toEqual([true, first.items[0]._cursor]);
    expect(first.items[0]).toMatchObject({ serverId: '9007199254740995', resourceType: 'image', sessionId: 'wxid_bob' });
    const second = await listResources(query, first.nextCursor, 1);
    expect(asked[1].searchParams.get('before')).toBe(first.nextCursor);
    expect([second.hasMore, second.nextCursor, second.items[0].fileName]).toEqual([false, null, 'report.pdf']);
    expect(await listResources(query, null, 1)).toEqual({ items: [], hasMore: false, nextCursor: null });
  });

  it('are the jobs of the task centre in each state, their times in milliseconds', async () => {
    process.env.TZ = 'UTC';
    serve(ok(SAMPLES.Job));
    const jobs: Job[] = await listJobs();
    expect(jobs.map(j => [j.status, isFinished(j.status), jobDetail(j)])).toEqual([
      ['queued', false, '排队中'],
      ['running', false, '1 / 3'],
      // a file whose path has no name
      ['succeeded', true, '文件 · 5.0 KB'],
      ['failed', true, 'too many reads at once'],
      ['cancelled', true, '已取消'],
    ]);
    expect(describeCommand(jobs[0].command)).toBe('导出聊天记录');
    expect(new Date(jobs[0].createdAt).toISOString()).toBe('2023-11-14T22:13:20.000Z');
  });

  it('fail with the code, the message and the details the server gave', async () => {
    const [usage, missing, busy] = SAMPLES.Error;
    serve(failed(400, usage), failed(404, missing), failed(503, busy));
    const problems: ApiError[] = [];
    for (let i = 0; i < 3; i++) await getMessages('wxid_bob', null).catch(problem => problems.push(problem));
    expect(problems.map(p => [p.status, p.code, p.message, p.details])).toEqual([
      [400, 'invalid_arguments', usage.message, { usage: 'weflow chat messages --session-id <SESSION_ID>' }],
      [404, 'runtime_error', 'the video has no thumbnail or cover', { failureKind: 'not_found' }],
      [503, 'busy', 'too many reads at once', undefined],
    ]);
  });
});
