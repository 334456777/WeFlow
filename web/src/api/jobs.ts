// Jobs (#211): commands run in the background with `Prefer: respond-async` — exports above all — their progress
// as server-sent events, cancelling them, and the file a `return=file` job wrote (docs/serve-api.md, "Jobs").
import { apiUrl, call, get, token, type Params } from './client';

export type JobStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled';

/** A command's `--progress` event. */
export interface JobProgress {
  stage?: string;
  message?: string;
  current?: number;
  total?: number;
  percent?: number;
}

export interface Job {
  id: string;
  status: JobStatus;
  /** The command line it runs, `weflow --json --progress …`. */
  command: string[];
  /** Unix milliseconds. */
  createdAt: number;
  startedAt: number | null;
  finishedAt: number | null;
  exitCode: number | null;
  httpStatus: number | null;
  progress: JobProgress | null;
  /** The file of a `return=file` job that succeeded. */
  file?: { name: string; size: number | null } | null;
  /** The command's `--json` document; only from `GET /jobs/<id>` and the `result` event. */
  result?: { success?: boolean; data?: Record<string, unknown>; error?: { code?: string; message?: string } } | null;
}

export const isFinished = (status: JobStatus) => status === 'succeeded' || status === 'failed' || status === 'cancelled';

/** Starts `method path` as a job; with `file`, the job keeps the file the command writes. */
export function startJob(method: 'POST' | 'GET', path: string, params: Params, file: boolean, signal?: AbortSignal): Promise<Job> {
  return call<Job>(method, path + (file ? '?return=file' : ''), params, signal, { Prefer: 'respond-async' });
}

export const listJobs = (signal?: AbortSignal) => get<Job[]>('jobs', {}, signal);

export const getJob = (id: string, signal?: AbortSignal) => get<Job>('jobs/' + encodeURIComponent(id), {}, signal);

/** Cancels a queued or running job, or forgets a finished one; answers the job as it ended. */
export const dropJob = (id: string) => call<Job>('DELETE', 'jobs/' + encodeURIComponent(id));

/** The job's file, for a link to save; the token goes in the address, as a link sends no header. */
export const jobFileUrl = (id: string) => apiUrl('jobs/' + encodeURIComponent(id) + '/file', { access_token: token() });

export const jobEventsUrl = (id: string) => apiUrl('jobs/' + encodeURIComponent(id) + '/events', { access_token: token() });

/** What a job does, in words: 「导出聊天记录」 for `export messages`, … */
export function describeCommand(command: string[]): string {
  const words = command.slice(1).filter(word => !word.startsWith('-'));
  const name = words.slice(0, 2).join(' ');
  return COMMANDS[name] ?? COMMANDS[words[0]] ?? words.slice(0, 2).join(' ');
}

const COMMANDS: Record<string, string> = {
  'export messages': '导出聊天记录',
  'export sessions': '导出会话列表',
  'export contacts': '导出通讯录',
  'export media': '导出媒体文件',
  'export footprint': '导出足迹',
  'backup create': '备份数据库',
  'backup restore': '恢复备份',
  'sns export': '导出朋友圈',
  'group export-members': '导出群成员',
  'group export-member-messages': '导出群成员的消息',
  key: '获取密钥',
  report: '生成报告',
};
