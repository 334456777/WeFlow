// `weflow serve` 的作业（#211），所有页面共用：`GET /jobs` 的列表、从事件流读到的正在运行的作业的进度，
// 以及这个标签页启动各个作业时给的名称。服务端默认一次只运行一个任务（`--max-tasks` 为 1），所以最多只开一个事件流；
// 有作业在排队时反复重读列表，从中看到排队的作业开始运行。页面放开长连接时（#799，见 presence.ts）
// 既不跟踪也不重读；重新可见后重读列表，重新跟踪正在运行的作业。
import { useEffect, useSyncExternalStore } from 'react';
import { ApiError, sampling, type Params } from '../api/client';
import { dropJob, getJob, isFinished, jobEventsUrl, listJobs, startJob, type Job } from '../api/jobs';
import { onPresence, present } from '../api/presence';
import { t } from '../i18n';

/** 有作业在排队或运行时，重读列表的间隔。 */
const POLL_MS = 2000;
/** 示例服务端的作业（#896）不发事件，它们的列表在页面里就能读到，所以重读得更频繁。 */
const SAMPLE_POLL_MS = 400;
/** 启动作业后多久重读列表，以便看到它离开队列。 */
const START_MS = 300;
const LABELS_KEY = 'weflow.jobs';

/** 服务端的作业，即任务中心显示的内容。 */
export interface JobsState {
  /** 最新的在前。 */
  jobs: Job[];
  /** 这个标签页启动作业时给它起的名称，如 `导出 林沐 · HTML`。 */
  labels: Record<string, string>;
  error?: string;
}

let state: JobsState = { jobs: [], labels: readLabels() };
const listeners = new Set<() => void>();
let users = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
let stream: { id: string; source: EventSource } | null = null;
let reading: Promise<void> | null = null;

function readLabels(): Record<string, string> {
  try { return JSON.parse(sessionStorage.getItem(LABELS_KEY) ?? '{}') as Record<string, string>; } catch { return {}; }
}

function set(next: Partial<JobsState>) {
  state = { ...state, ...next };
  if (next.labels) { try { sessionStorage.setItem(LABELS_KEY, JSON.stringify(next.labels)); } catch { /* 写不进去时，刷新页面后名称就没有了 */ } }
  for (const listener of [...listeners]) listener();
}

/** 回答是不是作业：其他种类的服务端可能回答任何内容。 */
const isJob = (value: unknown): value is Job =>
  typeof value === 'object' && value !== null && typeof (value as Job).id === 'string' && Array.isArray((value as Job).command);

/** 把 `job` 放进列表，替换它的旧副本并保留已经知道的结果；没有旧副本时加在最前面。 */
function merge(job: Job) {
  if (!isJob(job)) return;
  const old = state.jobs.find(j => j.id === job.id);
  const merged = { ...old, ...job, result: job.result ?? old?.result };
  set({ jobs: old ? state.jobs.map(j => (j.id === job.id ? merged : j)) : [merged, ...state.jobs] });
}

/** 重读列表，然后跟踪正在运行的作业，还有作业没结束时继续定期重读；同时调用时共用同一次读取。 */
export function refreshJobs(): Promise<void> {
  if (reading) return reading;
  reading = (async () => {
    try {
      const listed = await listJobs();
      const jobs = (Array.isArray(listed) ? listed : []).filter(isJob);
      const known = new Map(state.jobs.map(j => [j.id, j]));
      set({ jobs: [...jobs].reverse().map(job => ({ ...job, result: known.get(job.id)?.result })), error: undefined });
      // 列表里没有结果：另外读取这个标签页启动的作业的结果，以及失败的作业的原因
      const missing = state.jobs.filter(j => isFinished(j.status) && !j.result && (state.labels[j.id] || j.status === 'failed'));
      await Promise.all(missing.map(j => getJob(j.id).then(merge, () => {})));
    } catch (problem) {
      set({ error: (problem instanceof ApiError && problem.message) || t('读取任务列表失败') });
    } finally {
      reading = null;
    }
    follow();
  })();
  return reading;
}

/** 跟踪正在运行的作业；`soon` 表示刚启动了作业，很快就重读列表。没有页面在用或页面放开了长连接时关闭事件流。 */
function follow(soon = false) {
  clearTimeout(timer);
  if (!users || !present()) { closeStream(); return; }
  const running = state.jobs.find(j => j.status === 'running');
  const sample = sampling();
  if (stream && stream.id !== running?.id) closeStream();
  if (running && !stream && !sample && typeof EventSource !== 'undefined') openStream(running.id);
  if (state.jobs.some(j => !isFinished(j.status))) timer = setTimeout(() => { void refreshJobs(); }, soon ? START_MS : sample ? SAMPLE_POLL_MS : POLL_MS);
}

function openStream(id: string) {
  const source = new EventSource(jobEventsUrl(id));
  stream = { id, source };
  source.addEventListener('progress', event => {
    const job = state.jobs.find(j => j.id === id);
    try { if (job) merge({ ...job, status: 'running', progress: JSON.parse((event as MessageEvent).data) }); } catch { /* 不是 JSON */ }
  });
  source.addEventListener('result', event => {
    try { merge(JSON.parse((event as MessageEvent).data) as Job); } catch { /* 不是 JSON */ }
    closeStream();
    void refreshJobs();
  });
  // 事件流出错时关闭它：每隔几秒重读的列表仍会告诉作业怎样结束
  source.onerror = () => { if (stream?.source === source) closeStream(); };
}

function closeStream() {
  stream?.source.close();
  stream = null;
}

/** 以名称 `label` 把 `method path` 作为作业启动；`file` 为 true 时，作业成功后可以下载它写出的文件。 */
export async function runJob(label: string, method: 'POST' | 'GET', path: string, params: Params, file = true): Promise<Job> {
  const job = await startJob(method, path, params, file);
  set({ labels: { ...state.labels, [job.id]: label } });
  merge(job);
  follow(true);
  return job;
}

/** 取消没有结束的作业，或者让服务端忘掉已经结束的作业。 */
export async function removeJob(id: string): Promise<void> {
  const job = state.jobs.find(j => j.id === id);
  try {
    const ended = await dropJob(id);
    if (job && isFinished(job.status)) {
      const { [id]: _, ...labels } = state.labels;
      set({ jobs: state.jobs.filter(j => j.id !== id), labels });
    } else merge(ended);
  } catch (problem) {
    // 服务端已经忘掉了这个作业
    if (problem instanceof ApiError && problem.status === 404) set({ jobs: state.jobs.filter(j => j.id !== id) });
    else throw problem;
  }
  void refreshJobs();
}

const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
const snapshot = () => state;

/** `enabled` 期间读取的作业；换了服务端或账号（`scope`）时从空列表开始。 */
export function useJobs(enabled: boolean, scope: number): JobsState {
  useEffect(() => {
    if (!enabled) return;
    users += 1;
    set({ jobs: [], error: undefined });
    const off = onPresence(here => { if (here) void refreshJobs(); else follow(); });
    void refreshJobs();
    return () => { off(); users -= 1; follow(); };
  }, [enabled, scope]);
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/** 列表里 ID 为 `id` 的作业；没有时为 undefined。 */
export const findJob = (jobs: JobsState, id: string | null | undefined) => (id ? jobs.jobs.find(j => j.id === id) : undefined);
