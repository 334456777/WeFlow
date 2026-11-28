// 示例数据的作业（#896）：`weflow serve` 在后台运行的命令，在这里像一次只运行一个任务的服务端那样依次运行，
// 每个按页面的时钟运行几秒钟。写文件的作业交回一份说明文件，写明示例数据的记录不会导出，以及作业被要求做什么；
// 作业的改动（如保存密钥）在它成功后才生效。时间与服务端一样是 Unix 毫秒数。
import type { Job } from '../api/contract.gen';
import { threads } from './chats';
import { contacts, groups, nameOf, officials, sessionName } from './people';

type Params = Record<string, unknown>;

/** 作业成功后要做的改动，返回那时回答的数据。 */
export type Effect = () => unknown;

interface Note { name: string; text: string }

interface Run {
  job: Job;
  /** 作业的命令，如 `export messages`。 */
  name: string;
  params: Params;
  /** 运行多长时间（毫秒）。 */
  duration: number;
  /** 作业要处理的记录数，用于进度；不报告数量时为 0。 */
  total: number;
  /** 要求交回文件（`return=file`）时交回的说明文件。 */
  note: Note | null;
  effect?: Effect;
  /** 作业结束后它的 `--json` 输出。 */
  result: { success: boolean; data?: unknown } | null;
  url: string | null;
}

const runs: Run[] = [];
let counter = 0;

const list = (params: Params, key: string): string[] => {
  const value = params[key];
  return value === undefined || value === null || value === '' ? [] : Array.isArray(value) ? value.map(String) : [String(value)];
};
const text = (params: Params, key: string) => list(params, key)[0] ?? '';

/** 命令要处理的记录数，用于进度。 */
function workOf(name: string, params: Params): number {
  switch (name) {
    case 'export messages': return threads.get(text(params, 'session-id'))?.length ?? 0;
    case 'export contacts': return contacts().length;
    case 'export sessions': return threads.size + officials.length;
    case 'group export-members': return groups.find(g => g.username === text(params, 'chatroom-id'))?.members.length ?? 0;
    default: return 0;
  }
}

/** 命令运行多长时间：导出聊天记录的时长随消息数增加，其他命令是固定的时长。 */
function durationOf(name: string, total: number): number {
  switch (name) {
    case 'export messages': return Math.min(5000, 1000 + total * 15);
    case 'key db': return 3500;
    case 'backup create': return 4000;
    case 'backup restore': return 3000;
    default: return 1800;
  }
}

/** 运行中的作业说明自己在做什么，与命令的进度说明一样。 */
function messageOf(name: string): string {
  if (name === 'key db') return '等待微信打开数据库';
  if (name === 'backup create') return '正在复制数据库';
  if (name === 'backup restore') return '正在恢复';
  if (name.startsWith('export') || name.startsWith('group export')) return '正在导出';
  return '正在运行';
}

/** 用文字说明网页里各作业的命令。 */
const WHAT: Record<string, string> = {
  'export messages': '导出聊天记录', 'export sessions': '导出会话列表', 'export contacts': '导出通讯录', 'export footprint': '导出足迹',
  'group export-members': '导出群成员', 'group export-member-messages': '导出群成员的消息', 'backup create': '备份数据库',
};

/** 作业代替导出文件交回的说明文件：写明示例数据不导出，以及作业被要求做什么。 */
function noteOf(name: string, params: Params): Note {
  const about = text(params, 'session-id') || text(params, 'chatroom-id');
  const member = text(params, 'username');
  const days = [text(params, 'start'), text(params, 'end')];
  const asked = [
    '任务：' + (WHAT[name] ?? 'weflow ' + name) + (about ? ' · ' + sessionName(about) : '') + (member ? ' · ' + nameOf(member) : ''),
    list(params, 'format').length ? '格式：' + list(params, 'format').join('、') : '',
    days.some(Boolean) ? '时间范围：' + (days[0] || '最早') + ' 至 ' + (days[1] || '最新') : '',
    list(params, 'media').length ? '附带媒体：' + list(params, 'media').join('、') : '',
  ].filter(Boolean);
  return {
    name: 'WeFlow 示例数据说明.txt',
    text: ['这是 WeFlow 网页的示例数据，连接 weflow serve 后才能导出真实记录。', '', ...asked, ''].join('\n'),
  };
}

/** 作业运行的命令行：`weflow --json --progress export messages --session-id … --format html`。 */
function commandLine(name: string, params: Params): string[] {
  const flags: string[] = [];
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === false || value === '') continue;
    if (value === true) flags.push('--' + key);
    else for (const item of Array.isArray(value) ? value : [value]) flags.push('--' + key, String(item));
  }
  return ['weflow', '--json', '--progress', ...name.split(' '), ...flags];
}

const finished = (job: Job) => job.status === 'succeeded' || job.status === 'failed' || job.status === 'cancelled';

/** 把每个作业推进到 `now` 时的状态：前一个结束后开始，运行满时长后结束。 */
function advance(now = Date.now()) {
  let free = 0;
  for (const run of runs) {
    const { job } = run;
    if (finished(job)) { free = Math.max(free, job.finishedAt ?? 0); continue; }
    if (job.startedAt === null) {
      const start = Math.max(job.createdAt, free);
      if (now < start) { free = Number.POSITIVE_INFINITY; continue; }
      job.startedAt = start;
      job.status = 'running';
    }
    const end = job.startedAt + run.duration;
    if (now >= end) {
      complete(run, end);
      free = end;
      continue;
    }
    const share = Math.min(0.99, (now - job.startedAt) / run.duration);
    const formats = run.name === 'export messages' ? list(run.params, 'format') : [];
    job.progress = {
      stage: run.name, message: messageOf(run.name),
      ...(run.total ? { current: Math.round(run.total * share), total: run.total } : {}),
      percent: Math.round(share * 100),
      ...(formats.length > 1 ? { formats: formats.map((format, i) => ({ name: format, done: share >= (i + 1) / (formats.length + 0.5) })) } : {}),
    };
    free = Number.POSITIVE_INFINITY;
  }
}

function complete(run: Run, at: number) {
  const { job } = run;
  const data = run.effect?.() ?? (run.note ? { out: run.note.name } : {});
  job.status = 'succeeded';
  job.finishedAt = at;
  job.exitCode = 0;
  job.httpStatus = 200;
  job.progress = null;
  job.file = run.note ? { name: run.note.name, size: new TextEncoder().encode(run.note.text).length } : null;
  run.result = { success: true, data };
}

const copy = (run: Run, withResult: boolean): Job => {
  const { result: _, ...job } = run.job;
  return structuredClone(withResult ? { ...job, result: run.result } : job);
};

/**
 * 用 `params` 把 `name` 作为作业启动；`wantsFile` 时它交回说明文件供保存。`effect` 是它成功后要做的改动，
 * 返回那时回答的数据。
 */
export function startJob(name: string, params: Params, wantsFile: boolean, effect?: Effect): Job {
  const now = Date.now();
  advance(now);
  counter += 1;
  const total = workOf(name, params);
  const run: Run = {
    job: {
      id: 'sample-job-' + counter, status: 'queued', command: commandLine(name, params), createdAt: now, startedAt: null,
      finishedAt: null, exitCode: null, httpStatus: null, progress: null, file: null,
    },
    name, params, duration: durationOf(name, total), total, note: wantsFile ? noteOf(name, params) : null, effect, result: null, url: null,
  };
  runs.push(run);
  advance(now);
  return copy(run, false);
}

/** 全部作业，最早的在前，与 `GET /jobs` 列出的一样：不带结果。 */
export function listJobs(): Job[] {
  advance();
  return runs.map(run => copy(run, false));
}

/** 带结果的一个作业；没有这个 ID 的作业时为 null。 */
export function getJob(id: string): Job | null {
  advance();
  const run = runs.find(r => r.job.id === id);
  return run ? copy(run, true) : null;
}

/** 取消尚未结束的作业，或者删掉已经结束的作业；没有这个 ID 的作业时为 null。 */
export function dropJob(id: string): Job | null {
  advance();
  const index = runs.findIndex(r => r.job.id === id);
  if (index < 0) return null;
  const run = runs[index];
  if (finished(run.job)) {
    runs.splice(index, 1);
    if (run.url) URL.revokeObjectURL(run.url);
  } else {
    run.job.status = 'cancelled';
    run.job.finishedAt = Date.now();
    run.job.progress = null;
  }
  return copy(run, true);
}

/** 作业成功后说明文件的地址；只生成一次，这个对象 URL 在撤销之前一直有效。 */
export function jobFileUrl(id: string): string | null {
  advance();
  const run = runs.find(r => r.job.id === id);
  if (!run?.note || run.job.status !== 'succeeded') return null;
  run.url ??= URL.createObjectURL(new Blob([run.note.text], { type: 'text/plain;charset=utf-8' }));
  return run.url;
}
