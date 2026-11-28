// 示例服务端（#879）：尚无 `weflow serve` 回答过网页时，由它按命令 API 的格式回答（见 Wiki 的「命令 API」页），
// 各页面用显示服务端记录的同一套组件显示示例记录。改动保存在页面的内存里，刷新后复原（#896）：
// 设置、不参与统计的人、见解；每个作业运行几秒钟，导出交回一份说明文件，而不是导出的记录；
// 要访问微信或服务端所在电脑的操作，按在那里运行时的样子回答。
// 它是单独的 chunk，只在这时加载：除了类型，不从页面导入任何东西（plugins/plainImport.ts）。
import type { Health } from '../api/backend';
import type { SampleAnswer, SampleServer } from '../api/client';
import type { Answers, GroupMember, MemberAnalytics, Resource, ResourceKind, Session } from '../api/contract.gen';
import { excludeCandidates, excludedList, groupHours, groupList, groupMedia, groupMessages, groupRanking, memberMessages, overall, rankings, setExcluded, timeSpread } from './analytics';
import { backupManifest } from './backup';
import { dayOf, dayEnd, dayStart, NOW } from './clock';
import { messageOf, threads, unread, type Message } from './chats';
import {
  clearAccount, DATA_DIR, detectDirs, findAccounts, getSetting, planClearAccount, profileSummaries, saveDatabaseKey, saveImageKeys,
  setSetting, testDatabase, unsetSetting,
} from './config';
import { footprint, footprintSummary } from './footprint';
import { clearInsights, insightRecord, insightRecords, markRead, planClearInsights, sampleModels, testModel, triggerInsight } from './insights';
import { dropJob, getJob, jobFileUrl, listJobs, startJob, type Effect } from './jobs';
import { pictureUrl, voiceSound } from './media';
import { postCounts, timeline } from './moments';
import { pushes } from './official';
import { payRecords } from './pay';
import { contactRows, contacts, groups, ME, nameOf, officials, person, pinned, sessionName } from './people';
import { annualReport, dualReport, reportYears } from './reports';
import { clearCache, health, listCache } from './upkeep';

type Params = Parameters<SampleServer['answer']>[2];

/** 一个错误回答，相当于服务端带状态码的 `{ success: false, error }`。 */
class Failure {
  constructor(readonly status: number, readonly code: string, readonly message: string) {}
}

const text = (params: Params, key: string): string => {
  const value = params[key];
  return value === undefined || value === null ? '' : String(value);
};
const number = (params: Params, key: string): number | undefined => {
  const value = params[key];
  return value === undefined || value === null || value === '' ? undefined : Number(value);
};
const list = (params: Params, key: string): string[] => {
  const value = params[key];
  return value === undefined || value === null || value === '' ? [] : Array.isArray(value) ? value.map(String) : [String(value)];
};
const flag = (params: Params, key: string) => params[key] === true || params[key] === 'true';

/** 会话列表，最新的在前：各个聊天，以及按最后一次推送的时间排进来的公众号。 */
function sessions(): Session[] {
  const rows: Session[] = [];
  for (const [username, messages] of threads) {
    const last = messages[messages.length - 1];
    rows.push({
      username, type: 0, displayName: sessionName(username), summary: last.parsedContent, unreadCount: unread[username] ?? 0,
      sortTimestamp: last.createTime, lastTimestamp: last.createTime, lastMsgType: last.localType % 4_294_967_296,
      lastMsgSender: last.senderUsername ?? undefined, lastSenderDisplayName: last.senderUsername ? nameOf(last.senderUsername) : undefined,
      messageCountHint: messages.length, selfWxid: ME,
    });
  }
  for (const official of officials) {
    const latest = pushes.get(official.username)?.[0];
    if (!latest?.createTime) continue;
    rows.push({
      username: official.username, type: 0, displayName: official.name, summary: latest.title, unreadCount: 0,
      sortTimestamp: latest.createTime, lastTimestamp: latest.createTime, lastMsgType: 49, selfWxid: ME,
    });
  }
  return rows.sort((a, b) => b.sortTimestamp - a.sortTimestamp);
}

/** 会话的一页消息，最新的在前，与 `chat messages` 的读法一致：最新的、某个游标之前或之后的，或某个时间以后的。 */
function messagePage(params: Params): Message[] {
  const all = threads.get(text(params, 'session-id')) ?? [];
  const limit = number(params, 'limit') ?? 50;
  const at = (cursor: string) => all.findIndex(m => m._cursor === cursor);
  const before = text(params, 'before');
  const after = text(params, 'after');
  const since = number(params, 'since');
  let chosen: Message[];
  if (before) { const i = at(before); chosen = i < 0 ? [] : all.slice(Math.max(0, i - limit), i); }
  else if (after) { const i = at(after); chosen = i < 0 ? [] : all.slice(i + 1, i + 1 + limit); }
  else if (since !== undefined) { const i = all.findIndex(m => m.createTime >= since); chosen = i < 0 ? [] : all.slice(i, i + limit); }
  else chosen = all.slice(-limit);
  return chosen.slice().reverse();
}

const searchable = (m: Message) => [m.parsedContent, m.fileName, m.linkTitle, m.appMsgDesc, m.quotedContent].filter(Boolean).join('\n').toLocaleLowerCase();

function search(params: Params) {
  const keyword = text(params, 'keyword').trim().toLocaleLowerCase();
  const only = text(params, 'session');
  const start = number(params, 'start') ?? 0;
  const end = number(params, 'end') ?? 0;
  const offset = number(params, 'offset') ?? 0;
  const limit = number(params, 'limit') ?? 20;
  if (!keyword) return [];
  const hits: (Message & { _session_id: string })[] = [];
  for (const [session, messages] of threads) {
    if (only && session !== only) continue;
    for (const m of messages) {
      if (m.localType === 10000 || (start && m.createTime < start) || (end && m.createTime > end)) continue;
      if (searchable(m).includes(keyword)) hits.push({ ...m, _session_id: session });
    }
  }
  return hits.sort((a, b) => b.createTime - a.createTime).slice(offset, offset + limit);
}

function dateCounts(params: Params) {
  const sessionId = text(params, 'session-id');
  const counts: Record<string, number> = {};
  for (const m of threads.get(sessionId) ?? []) counts[dayOf(m.createTime)] = (counts[dayOf(m.createTime)] ?? 0) + 1;
  return { sessionId, counts };
}

const BASE = 4_294_967_296;
const resourceKind = (m: Message): ResourceKind | null =>
  m.localType === 3 ? 'image' : m.localType === 34 ? 'voice' : m.localType === 43 ? 'video' : m.localType % BASE === 49 && m.appMsgKind === 'file' ? 'file' : null;

/** 各个会话里的全部图片、语音、视频和文件消息，最新的在前。 */
function resources(): Resource[] {
  const all: Resource[] = [];
  for (const [session, messages] of threads) {
    for (const m of messages) {
      const kind = resourceKind(m);
      if (!kind) continue;
      all.push({
        localId: m.localId, serverId: m.serverId, createTime: m.createTime, localType: m.localType, isSend: m.isSend ?? 0,
        senderUsername: m.senderUsername ?? '', text: m.parsedContent, imageMd5: m.imageMd5, appMsgKind: m.appMsgKind,
        content: kind === 'voice' ? `<msg><voicemsg voicelength="${(m.voiceDurationSeconds ?? 0) * 1000}" /></msg>` : '',
        fileName: m.fileName, fileExt: m.fileExt, fileSize: m.fileSize,
        _cursor: session + ':' + m.localId, sessionDisplayName: sessionName(session), sessionId: session, resourceType: kind,
      });
    }
  }
  return all.sort((a, b) => b.createTime - a.createTime);
}

function resourcePage(params: Params) {
  const session = text(params, 'session');
  const types = list(params, 'type');
  const start = text(params, 'start');
  const end = text(params, 'end');
  const limit = number(params, 'limit') ?? 60;
  const chosen = resources().filter(r => (!session || r.sessionId === session) && (!types.length || types.includes(r.resourceType))
    && (!start || r.createTime >= dayStart(start)) && (!end || r.createTime <= dayEnd(end)));
  const before = text(params, 'before');
  const from = before ? chosen.findIndex(r => r._cursor === before) + 1 : 0;
  const items = chosen.slice(from, from + limit);
  const hasMore = from + limit < chosen.length;
  return { items, hasMore, nextCursor: hasMore ? items[items.length - 1]._cursor : null };
}

function detail(params: Params) {
  const wxid = text(params, 'session-id');
  const p = person(wxid);
  return {
    wxid, displayName: sessionName(wxid), remark: p?.remark, nickName: p?.nickname ?? sessionName(wxid), alias: p?.alias,
    messageCount: threads.get(wxid)?.length ?? 0,
  };
}

function detailExtra(params: Params) {
  const messages = threads.get(text(params, 'session-id')) ?? [];
  const hash = [...text(params, 'session-id')].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7).toString(16).padStart(8, '0');
  return {
    firstMessageTime: messages[0]?.createTime ?? null, latestMessageTime: messages[messages.length - 1]?.createTime ?? null,
    messageTables: messages.length ? [{ dbName: 'message_0.db', tableName: 'Msg_' + hash.repeat(4), count: messages.length }] : [],
  };
}

function members(params: Params) {
  const chatroomId = text(params, 'chatroom-id');
  const group = groups.find(g => g.username === chatroomId);
  const messages = threads.get(chatroomId) ?? [];
  const rows: GroupMember[] = (group?.members ?? []).map(username => {
    const p = person(username);
    return {
      username, displayName: p?.name ?? username, nickname: p?.nickname ?? '', alias: p?.alias ?? '', remark: p?.remark ?? '',
      groupNickname: '', isOwner: group?.owner === username, isFriend: username !== ME && !p?.formerFriend,
      messageCount: messages.filter(m => m.senderUsername === username).length,
    };
  });
  return { chatroomId, count: rows.length, updatedAt: NOW, members: rows.sort((a, b) => b.messageCount - a.messageCount) };
}

/** 一个成员在群里的消息，按 `group member` 的方式统计。 */
function memberStats(params: Params): MemberAnalytics {
  const username = text(params, 'username');
  const said = groupMessages(text(params, 'chatroom-id'), text(params, 'start'), text(params, 'end')).filter(m => m.senderUsername === username);
  const of = (type: number) => said.filter(m => m.localType % BASE === type).length;
  const hours: Record<string, number> = {};
  const types: Record<string, number> = {};
  for (const m of said) {
    const hour = String(new Date(m.createTime * 1000).getHours());
    hours[hour] = (hours[hour] ?? 0) + 1;
    types[String(m.localType % BASE)] = (types[String(m.localType % BASE)] ?? 0) + 1;
  }
  const texts = said.filter(m => m.localType === 1).map(m => m.parsedContent);
  const phrases = (person(username)?.phrases ?? [])
    .map(phrase => ({ phrase, count: texts.filter(t => t.includes(phrase)).length }))
    .filter(p => p.count > 0).sort((a, b) => b.count - a.count);
  const emojis: Record<string, number> = {};
  for (const t of texts) for (const e of t.match(/\p{Extended_Pictographic}/gu) ?? []) emojis[e] = (emojis[e] ?? 0) + 1;
  const days = new Set(said.map(m => dayOf(m.createTime)));
  const textCount = of(1);
  const images = of(3);
  const voices = of(34);
  const videos = of(43);
  const stickers = of(47);
  return {
    statistics: {
      totalMessages: said.length, textMessages: textCount, imageMessages: images, voiceMessages: voices, videoMessages: videos,
      emojiMessages: stickers, otherMessages: said.length - textCount - images - voices - videos - stickers,
      firstMessageTime: said[0]?.createTime ?? null, lastMessageTime: said[said.length - 1]?.createTime ?? null,
      activeDays: days.size, messageTypeCounts: types,
    },
    timeDistribution: hours,
    commonPhrases: phrases,
    commonEmojis: Object.entries(emojis).map(([emoji, count]) => ({ emoji, count })).sort((a, b) => b.count - a.count),
  };
}

function paged<T>(items: readonly T[], params: Params, size: number): T[] {
  const offset = number(params, 'offset') ?? 0;
  return items.slice(offset, offset + (number(params, 'limit') ?? size));
}

function snsTimeline(params: Params) {
  const user = text(params, 'user');
  const keyword = text(params, 'keyword').trim();
  const start = number(params, 'start') ?? 0;
  const end = number(params, 'end') ?? 0;
  const chosen = timeline.filter(p => (!user || p.username === user) && (!keyword || p.contentDesc.includes(keyword))
    && (!start || p.createTime >= start) && (!end || p.createTime <= end));
  return { timeline: paged(chosen, params, 20) };
}

type Route = (params: Params) => unknown;

/** 一个路由运行的命令，按服务端的叫法：`GET api/chat/sessions` 运行 `chat sessions`。 */
type CommandOf<Key> = Key extends `${string} ${infer Path}` ? Words<Path extends `api/${infer Read}` ? Read : Path> : never;
/** 把路由的路径写成命令的词：`chat/sessions` 写成 `chat sessions`。 */
type Words<Path extends string> = Path extends `${infer Head}/${infer Tail}` ? `${Head} ${Words<Tail>}` : Path;

/**
 * 一个路由可以回答的内容：契约里它的命令的 `data`，或者一个失败。`health` 按网页读取它的方式回答，
 * `config get` 回答设置的值，这两者契约里没有描述；其他命令都必须在契约里。
 */
type Answer<Key> = Failure | (Key extends 'GET health' ? Health : Key extends 'GET api/config/get' ? string | null
  : CommandOf<Key> extends keyof Answers ? Answers[CommandOf<Key>] : never);

/** 路由表，每个路由都按契约里它的命令的回答类型检查（#1267）。 */
const described = <T extends Record<string, Route>>(table: T & NoInfer<{ [Key in keyof T]: (params: Params) => Answer<Key> }>): T => table;

/** 示例服务端回答的路由，按方法和路径列出。 */
const table = {
  'GET api/chat/sessions': () => sessions(),
  'GET api/chat/statuses': params => Object.fromEntries(list(params, 'sessions').map(username => {
    const group = groups.find(g => g.username === username);
    return [username, { isFolded: Boolean(group?.folded), isMuted: Boolean(group?.muted), isPinned: pinned.has(username) }];
  })),
  'GET api/chat/messages': messagePage,
  'GET api/chat/search': search,
  'GET api/chat/date-counts': dateCounts,
  'GET api/chat/message': params => messageOf(text(params, 'session-id'), number(params, 'local-id') ?? 0) ?? new Failure(404, 'runtime_error', '未找到该消息'),
  'GET api/chat/image-info': params => {
    const localId = number(params, 'local-id') ?? 0;
    return { thumb: true, normal: true, hd: localId % 2 === 0, normalBytes: 180_000 + localId * 1_000, hdBytes: localId % 2 === 0 ? 2_400_000 + localId * 10_000 : null };
  },
  'GET api/chat/detail': detail,
  'GET chat/detail': detailExtra,
  'GET api/chat/resources': resourcePage,
  'GET api/chat/contacts': params => (flag(params, 'parsed') ? contacts() : contactRows()),
  'GET api/group/members': members,
  'GET api/group/member': memberStats,
  'GET api/biz/accounts': () => ({
    accounts: officials.map(o => ({ username: o.username, nick_name: o.name, lastTime: pushes.get(o.username)?.[0]?.createTime ?? 0 })),
  }),
  'GET api/biz/messages': params => ({ username: text(params, 'username'), messages: paged(pushes.get(text(params, 'username')) ?? [], params, 20) }),
  'GET api/biz/pay-records': params => {
    const offset = number(params, 'offset') ?? 0;
    const limit = number(params, 'limit') ?? 500;
    return { records: payRecords.slice(offset, offset + limit), hasMore: offset + limit < payRecords.length };
  },
  'GET api/sns/timeline': snsTimeline,
  'GET api/sns/post-counts': () => postCounts(),
  // 发过动态的人数，包括账号本人，与服务端的计法一致（#1158）
  'GET api/sns/stats': () => ({ totalPosts: timeline.length, totalFriends: Object.keys(postCounts()).length, myPosts: postCounts()[ME] ?? 0 }),
  'GET api/analytics/overall': () => overall(),
  'GET api/analytics/rankings': params => rankings(number(params, 'limit') ?? 20),
  'GET api/analytics/time': () => timeSpread(),
  'GET api/analytics/excluded': () => excludedList(),
  'GET api/analytics/exclude-candidates': () => excludeCandidates(),
  'PUT analytics/excluded': params => setExcluded(list(params, 'set')),
  'GET api/group/list': () => groupList(),
  'GET api/group/ranking': params => groupRanking(text(params, 'chatroom-id'), text(params, 'start'), text(params, 'end'), number(params, 'limit') ?? 20),
  'GET api/group/hours': params => groupHours(text(params, 'chatroom-id'), text(params, 'start'), text(params, 'end')),
  'GET api/group/media': params => groupMedia(text(params, 'chatroom-id'), text(params, 'start'), text(params, 'end')),
  'GET api/group/member-messages': params => memberMessages(text(params, 'chatroom-id'), text(params, 'username'), text(params, 'start'), text(params, 'end'),
    number(params, 'cursor') ?? 0, number(params, 'limit') ?? 50),
  'GET api/report/annual/years': () => reportYears(),
  // 报告是生成出来的，命令 API 把它当作 POST（#336）：实际上不改变任何东西
  'POST report/annual/generate': params => annualReport(number(params, 'year') ?? 0),
  'POST report/dual/generate': params => dualReport(text(params, 'friend'), number(params, 'year') ?? 0),
  'GET api/insight/footprint': params => footprint(text(params, 'start'), text(params, 'end')),
  'POST insight/footprint-summary': params => footprintSummary(text(params, 'payload-json')),
  'GET insight/records': params => insightRecords(text(params, 'keyword'), text(params, 'session'), number(params, 'offset') ?? 0, number(params, 'limit') ?? 50),
  'GET api/insight/get': params => insightRecord(text(params, 'id')) ?? new Failure(404, 'not_found', '未找到该见解记录'),
  'PUT insight/mark-read': params => markRead(text(params, 'id')) ?? new Failure(404, 'not_found', '未找到该见解记录'),
  'DELETE insight/clear': params => (flag(params, 'dry-run') ? planClearInsights : clearInsights)(text(params, 'session')),
  'POST insight/trigger': params => {
    const reply = triggerInsight(text(params, 'session-id'));
    return 'problem' in reply ? new Failure(500, 'runtime_error', reply.problem) : reply;
  },
  'POST insight/test': () => testModel(getSetting('ai_model_api_format'), getSetting('ai_model_api_model')),
  'GET insight/models': () => sampleModels(getSetting('ai_model_api_format')),
  'GET backup/inspect': () => backupManifest(),

  // 设置、账号设置和服务端自身的维护（#896）
  'GET health': () => health(),
  'GET api/config/profiles': () => profileSummaries(),
  'GET api/config/get': params => getSetting(text(params, 'key')),
  'PUT config/set': params => setSetting(text(params, 'key'), text(params, 'value')),
  'DELETE config/unset': params => unsetSetting(text(params, 'key')),
  'GET api/db/detect': () => detectDirs(),
  'GET api/db/wxid': params => findAccounts(text(params, 'root') || DATA_DIR),
  'GET api/db/test': () => {
    const tested = testDatabase();
    return 'problem' in tested ? new Failure(400, 'config_error', tested.problem) : tested.data;
  },
  'POST key/image': () => saveImageKeys(),
  'DELETE cache/clear-account': params => (flag(params, 'dry-run') ? planClearAccount() : clearAccount()),
  'GET api/cache/list': () => listCache(),
  'DELETE cache/clear': params => clearCache(params),
} satisfies Record<string, Route>;

/** 示例服务端回答的路由，每个都按契约的规定回答（#1267）。 */
const routes: Record<string, Route> = described(table);

/** 示例数据里备份写到的位置。 */
const BACKUP = 'C:\\Users\\sample\\Documents\\WeFlow\\backups\\weflow-backup.zip';

/** `command` 的作业成功后要做的改动，以及那时回答的数据。 */
function effectOf(command: string, params: Params): Effect | undefined {
  switch (command) {
    case 'key db': return () => saveDatabaseKey();
    case 'backup create': return () => ({ out: BACKUP, skipped: backupManifest().skipped, unsettled: [] });
    case 'backup restore': return () => ({ restoredTo: text(params, 'out') });
    default: return undefined;
  }
}

/** 路由对应的命令：`export/messages` 是 `export messages`。 */
const commandOf = (route: string) => route.split('/').join(' ');

/** 回答 `method route`：路由的数据或失败；示例服务端没有这个路由时为 `refused`。 */
function answer(method: string, route: string, params: Params, async: boolean, wantsFile: boolean): SampleAnswer {
  if (method === 'GET' && route === 'jobs') return { data: listJobs() };
  const job = /^jobs\/([^/]+)$/.exec(route);
  if (job && (method === 'GET' || method === 'DELETE')) {
    const id = decodeURIComponent(job[1]);
    const found = method === 'GET' ? getJob(id) : dropJob(id);
    return found ? { data: found } : { failed: { status: 404, code: 'not_found', message: '没有这个任务。' } };
  }
  if (async && method !== 'GET') return { data: startJob(commandOf(route), params, wantsFile, effectOf(commandOf(route), params)) };
  const handler = routes[method + ' ' + route];
  if (!handler) return { refused: true };
  const data = handler(params);
  return data instanceof Failure ? { failed: { status: data.status, code: data.code, message: data.message } } : { data };
}

/** 没有服务端回答时 `api/client.ts` 加载的示例服务端：它的路由和媒体文件的地址。 */
export const server: SampleServer = {
  answer(method, path, params, async) {
    const [route, query = ''] = path.split('?');
    return answer(method, route, params, async, new URLSearchParams(query).get('return') === 'file');
  },
  media(path, params) {
    if (path === 'api/chat/voice-data') return voiceSound();
    if (path === 'api/chat/image-data') return pictureUrl(messageOf(text(params, 'session-id'), number(params, 'local-id') ?? 0)?.imageMd5 ?? '');
    if (path === 'api/sns/media') return pictureUrl(text(params, 'url').split('/').pop() ?? '');
    const job = /^jobs\/([^/]+)\/file$/.exec(path);
    if (job) return jobFileUrl(decodeURIComponent(job[1])) ?? 'about:blank';
    return pictureUrl('logo');
  },
};
