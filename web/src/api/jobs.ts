// 作业（#211）：带 `Prefer: respond-async` 在后台运行的命令（主要是导出）：用服务器发送事件（SSE）报告的进度、
// 取消作业，以及 `return=file` 的作业写出的文件（见 Wiki 的「命令 API」页）。
import { apiUrl, call, get, mediaAddress, token, type Params } from './client';
import type * as Contract from './contract.gen';
import { formatSize } from '../format';
import { msg, t } from '../i18n';

/** 作业的状态：排队中、正在运行，或者以什么方式结束。 */
export type JobStatus = Contract.JobStatus;

/** 命令的一个 `--progress` 事件。 */
export interface JobProgress {
  stage?: string;
  message?: string;
  current?: number;
  total?: number;
  percent?: number;
  /** 同时写出的几种导出格式（#628），以及其中哪些已经写完。 */
  formats?: { name: string; done: boolean }[];
}

/** 一个作业（契约的类型，#399），带上网页从它最后一个进度事件和结果里读取的内容。 */
export type Job = Omit<Contract.Job, 'progress' | 'result'> & {
  progress: JobProgress | null;
  /** 命令的 `--json` 输出；只有 `GET /jobs/<id>` 和 `result` 事件里有。 */
  result?: { success?: boolean; data?: Record<string, unknown>; error?: { code?: string; message?: string } } | null;
};

/** 作业是否已经结束：成功、失败或已取消。 */
export const isFinished = (status: JobStatus) => status === 'succeeded' || status === 'failed' || status === 'cancelled';

// 显示时经过 t()
const STATUS: Record<Job['status'], string> = { queued: msg('排队中'), running: msg('正在运行'), succeeded: msg('已完成'), failed: msg('未完成'), cancelled: msg('已取消') };

/** 作业名称下面那一行的文字：进度、错误，或写出的文件。 */
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

/** 作业的状态加上它的说明，如「未完成 · 磁盘空间不足」。 */
export const jobLine = (job: Job) => {
  const status = t(STATUS[job.status]);
  return status === jobDetail(job) ? status : status + ' · ' + jobDetail(job);
};

/** 把 `method path` 作为作业启动；`file` 为 true 时，作业保留命令写出的文件。 */
export function startJob(method: 'POST' | 'GET', path: string, params: Params, file: boolean, signal?: AbortSignal): Promise<Job> {
  return call<Job>(method, path + (file ? '?return=file' : ''), params, signal, { Prefer: 'respond-async' });
}

/** `GET /jobs`：服务端保存的作业。 */
export const listJobs = (signal?: AbortSignal) => get<Job[]>('jobs', {}, signal);

/** `GET /jobs/<id>`：一个作业，结束后带上它的结果。 */
export const getJob = (id: string, signal?: AbortSignal) => get<Job>('jobs/' + encodeURIComponent(id), {}, signal);

/** `DELETE /jobs/<id>`：取消排队中或正在运行的作业，或者删掉已经结束的作业；回答作业结束时的样子。 */
export const dropJob = (id: string) => call<Job>('DELETE', 'jobs/' + encodeURIComponent(id));

/**
 * 作业写出的文件的地址，供保存文件的链接使用；链接不能带请求头，所以 token 放在地址里。
 * 示例数据的作业（#896）把文件保存在页面里。
 */
export const jobFileUrl = (id: string) => mediaAddress('jobs/' + encodeURIComponent(id) + '/file', { access_token: token() });

/** 作业的服务器发送事件的地址；`EventSource` 不能带请求头，所以 token 放在地址里。 */
export const jobEventsUrl = (id: string) => apiUrl('jobs/' + encodeURIComponent(id) + '/events', { access_token: token() });

/** 作业命令行里不是选项的词，去掉 `weflow`：先是 `export`、`messages`，然后是参数的值。 */
const wordsOf = (command: readonly string[]) => command.slice(1).filter(word => !word.startsWith('-'));

/** 作业运行的命令，取前两个词：`export messages`、`key db`。 */
export const commandName = (command: readonly string[]) => wordsOf(command).slice(0, 2).join(' ');

/**
 * 用文字说明作业做什么：`export messages` 为「导出聊天记录」，等等。
 * 给了 `names`（会话 ID 到名称）时加上命令里的会话，如「导出聊天记录 · 林沐」。
 */
export function describeCommand(command: string[], names?: ReadonlyMap<string, string>): string {
  const words = wordsOf(command);
  const name = commandName(command);
  const known = COMMANDS[name] ?? COMMANDS[words[0]];
  const what = known ? t(known) : name;
  const about = names && command.find(word => names.has(word));
  return about ? what + ' · ' + names.get(about) : what;
}

// 显示时经过 t()
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
