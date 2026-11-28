// Jobs (#211): commands run in the background with `Prefer: respond-async` — exports above all — their progress
// as server-sent events, cancelling them, and the file a `return=file` job wrote (the wiki's command API page).
import { apiUrl, call, get, mediaAddress, token, type Params } from './client';
import type * as Contract from './contract.gen';
import { formatSize } from '../format';
import { msg, t } from '../i18n';

/** Where a job stands: queued, running, or how it ended. */
export type JobStatus = Contract.JobStatus;

/** A command's `--progress` event. */
export interface JobProgress {
  stage?: string;
  message?: string;
  current?: number;
  total?: number;
  percent?: number;
  /** The formats of an export written side by side (#628), and which of them are written. */
  formats?: { name: string; done: boolean }[];
}

/** A job (the contract's, #399), with what the page reads of its last progress event and of its result. */
export type Job = Omit<Contract.Job, 'progress' | 'result'> & {
  progress: JobProgress | null;
  /** The command's `--json` document; only from `GET /jobs/<id>` and the `result` event. */
  result?: { success?: boolean; data?: Record<string, unknown>; error?: { code?: string; message?: string } } | null;
};

/** Whether the job has ended: succeeded, failed or cancelled. */
export const isFinished = (status: JobStatus) => status === 'succeeded' || status === 'failed' || status === 'cancelled';

// shown through t()
const STATUS: Record<Job['status'], string> = { queued: msg('排队中'), running: msg('正在运行'), succeeded: msg('已完成'), failed: msg('未完成'), cancelled: msg('已取消') };

/** What the line under a job's name says: its progress, its error, or its file. */
export function jobDetail(job: Job): string {
  if (job.status === 'running') {
    const p = job.progress;
    if (p?.formats?.every(f => f.done)) return t('正在打包');
    const count = p?.total ? (p.current ?? 0) + ' / ' + p.total : '';
    return [p?.message, count, p?.percent !== undefined ? Math.round(p.percent) + '%' : ''].filter(Boolean).join(' · ') || t('正在运行');
  }
  if (job.status === 'failed') return job.result?.error?.message || (job.httpStatus ? t('命令没有完成（HTTP {status}）', { status: job.httpStatus }) : t('命令没有完成'));
  if (job.status === 'succeeded' && job.file) return (job.file.name ?? t('文件')) + (job.file.size !== null ? ' · ' + formatSize(job.file.size) : '');
  return t(STATUS[job.status]);
}

/** The state of a job and what it says: 「未完成 · 磁盘空间不足」. */
export const jobLine = (job: Job) => {
  const status = t(STATUS[job.status]);
  return status === jobDetail(job) ? status : status + ' · ' + jobDetail(job);
};

/** Starts `method path` as a job; with `file`, the job keeps the file the command writes. */
export function startJob(method: 'POST' | 'GET', path: string, params: Params, file: boolean, signal?: AbortSignal): Promise<Job> {
  return call<Job>(method, path + (file ? '?return=file' : ''), params, signal, { Prefer: 'respond-async' });
}

/** `GET /jobs`: the jobs the server keeps. */
export const listJobs = (signal?: AbortSignal) => get<Job[]>('jobs', {}, signal);

/** `GET /jobs/<id>`: one job, with its result once it ended. */
export const getJob = (id: string, signal?: AbortSignal) => get<Job>('jobs/' + encodeURIComponent(id), {}, signal);

/** Cancels a queued or running job, or forgets a finished one; answers the job as it ended. */
export const dropJob = (id: string) => call<Job>('DELETE', 'jobs/' + encodeURIComponent(id));

/**
 * The job's file, for a link to save; the token goes in the address, as a link sends no header. The sample's jobs
 * (#896) keep theirs in the page.
 */
export const jobFileUrl = (id: string) => mediaAddress('jobs/' + encodeURIComponent(id) + '/file', { access_token: token() });

/** The address of the job's server-sent events; the token goes in the address, as `EventSource` sends no header. */
export const jobEventsUrl = (id: string) => apiUrl('jobs/' + encodeURIComponent(id) + '/events', { access_token: token() });

/** The words of a job's command line that are not flags, `weflow` left out: `export`, `messages`, then the values. */
const wordsOf = (command: readonly string[]) => command.slice(1).filter(word => !word.startsWith('-'));

/** The command a job runs, by its first two words: `export messages`, `key db`. */
export const commandName = (command: readonly string[]) => wordsOf(command).slice(0, 2).join(' ');

/**
 * What a job does, in words: 「导出聊天记录」 for `export messages`, … With `names` (conversations by id), the
 * conversation it names: 「导出聊天记录 · 林沐」.
 */
export function describeCommand(command: string[], names?: ReadonlyMap<string, string>): string {
  const words = wordsOf(command);
  const name = commandName(command);
  const known = COMMANDS[name] ?? COMMANDS[words[0]];
  const what = known ? t(known) : name;
  const about = names && command.find(word => names.has(word));
  return about ? what + ' · ' + names.get(about) : what;
}

// shown through t()
const COMMANDS: Record<string, string> = {
  'export messages': msg('导出聊天记录'),
  'export sessions': msg('导出会话列表'),
  'export contacts': msg('导出通讯录'),
  'export media': msg('导出媒体文件'),
  'export footprint': msg('导出足迹'),
  'backup create': msg('备份数据库'),
  'backup restore': msg('恢复备份'),
  'sns export': msg('导出朋友圈'),
  'group export-members': msg('导出群成员'),
  'group export-member-messages': msg('导出群成员的消息'),
  key: msg('获取密钥'),
  report: msg('生成报告'),
};
