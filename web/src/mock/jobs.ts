// The sample's jobs (#896): the commands `weflow serve` runs in the background, run here one after another as a server
// running one task at a time runs them, each for a few seconds of the page's clock. A job that writes a file hands back
// a note saying that the sample's records are not exported, and what the job was asked for; what a job changes (a key
// saved) is done once it has succeeded. Times are Unix milliseconds, as the server's.
import type { Job } from '../api/contract.gen';
import { threads } from './chats';
import { contacts, groups, nameOf, officials, sessionName } from './people';

type Params = Record<string, unknown>;

/** What a job changes once it has succeeded, and the data it answers then. */
export type Effect = () => unknown;

interface Note { name: string; text: string }

interface Run {
  job: Job;
  /** Its command, `export messages`. */
  name: string;
  params: Params;
  /** How long it runs. */
  duration: number;
  /** The records it goes through, for its progress; 0 when it does not say. */
  total: number;
  /** The note it hands back, when it was asked for its file (`return=file`). */
  note: Note | null;
  effect?: Effect;
  /** The document of its `--json` output once it has ended. */
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

/** The records a command goes through, for its progress. */
function workOf(name: string, params: Params): number {
  switch (name) {
    case 'export messages': return threads.get(text(params, 'session-id'))?.length ?? 0;
    case 'export contacts': return contacts().length;
    case 'export sessions': return threads.size + officials.length;
    case 'group export-members': return groups.find(g => g.username === text(params, 'chatroom-id'))?.members.length ?? 0;
    default: return 0;
  }
}

/** How long a command runs: an export by the messages it goes through, the rest a fixed while. */
function durationOf(name: string, total: number): number {
  switch (name) {
    case 'export messages': return Math.min(5000, 1000 + total * 15);
    case 'key db': return 3500;
    case 'backup create': return 4000;
    case 'backup restore': return 3000;
    default: return 1800;
  }
}

/** What a running job says it is doing, as the commands' progress says. */
function messageOf(name: string): string {
  if (name === 'key db') return '等待微信打开数据库';
  if (name === 'backup create') return '正在复制数据库';
  if (name === 'backup restore') return '正在恢复';
  if (name.startsWith('export') || name.startsWith('group export')) return '正在导出';
  return '正在运行';
}

/** The commands of the page's jobs, in words. */
const WHAT: Record<string, string> = {
  'export messages': '导出聊天记录', 'export sessions': '导出会话列表', 'export contacts': '导出通讯录', 'export footprint': '导出足迹',
  'group export-members': '导出群成员', 'group export-member-messages': '导出群成员的消息', 'backup create': '备份数据库',
};

/** The note a job hands back in place of its file: that the sample is not exported, and what was asked for. */
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

/** The command line a job runs: `weflow --json --progress export messages --session-id … --format html`. */
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

/** Moves every job on to where it is at `now`: started once the one before has ended, ended once it has run its time. */
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
 * Starts `name` with `params` as a job; with `wantsFile`, it hands back its note to be saved. `effect` is what it
 * changes once it has succeeded, and the data it answers then.
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

/** Every job, oldest first, as `GET /jobs` lists them: without their results. */
export function listJobs(): Job[] {
  advance();
  return runs.map(run => copy(run, false));
}

/** One job with its result; null when there is none of that id. */
export function getJob(id: string): Job | null {
  advance();
  const run = runs.find(r => r.job.id === id);
  return run ? copy(run, true) : null;
}

/** Cancels a job that has not ended, or forgets one that has; null when there is none of that id. */
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

/** The address of a job's note, once it has succeeded; made once, as the browser keeps it. */
export function jobFileUrl(id: string): string | null {
  advance();
  const run = runs.find(r => r.job.id === id);
  if (!run?.note || run.job.status !== 'succeeded') return null;
  run.url ??= URL.createObjectURL(new Blob([run.note.text], { type: 'text/plain;charset=utf-8' }));
  return run.url;
}
