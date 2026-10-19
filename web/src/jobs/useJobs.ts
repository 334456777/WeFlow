// The jobs of `weflow serve` (#211), shared by every page: the list of `GET /jobs`, the progress of the running
// one from its event stream, and what this tab started them for. Tasks run one at a time on the server, so one
// stream is open at most; the queued ones are seen to start by reading the list again while any wait.
import { useEffect, useSyncExternalStore } from 'react';
import { ApiError, type Params } from '../api/client';
import { dropJob, getJob, isFinished, jobEventsUrl, listJobs, startJob, type Job } from '../api/jobs';

/** How often the list is read again while a job waits or runs. */
const POLL_MS = 2000;
const LABELS_KEY = 'weflow.jobs';

export interface JobsState {
  /** Newest first. */
  jobs: Job[];
  /** The name this tab gave a job when it started it: 「导出 林沐 · HTML」. */
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
  if (next.labels) { try { sessionStorage.setItem(LABELS_KEY, JSON.stringify(next.labels)); } catch { /* lost on reload */ } }
  for (const listener of [...listeners]) listener();
}

/** Whether an answer is a job, as a server of another kind could send anything. */
const isJob = (value: unknown): value is Job =>
  typeof value === 'object' && value !== null && typeof (value as Job).id === 'string' && Array.isArray((value as Job).command);

/** Puts `job` in the list in place of its older copy, keeping the result already known. */
function merge(job: Job) {
  if (!isJob(job)) return;
  const old = state.jobs.find(j => j.id === job.id);
  const merged = { ...old, ...job, result: job.result ?? old?.result };
  set({ jobs: old ? state.jobs.map(j => (j.id === job.id ? merged : j)) : [merged, ...state.jobs] });
}

/** Reads the list again, then follows the running job and keeps reading while any job is not finished. */
export function refreshJobs(): Promise<void> {
  if (reading) return reading;
  reading = (async () => {
    try {
      const listed = await listJobs();
      const jobs = (Array.isArray(listed) ? listed : []).filter(isJob);
      const known = new Map(state.jobs.map(j => [j.id, j]));
      set({ jobs: [...jobs].reverse().map(job => ({ ...job, result: known.get(job.id)?.result })), error: undefined });
      // the result of a job this tab started, when it ended between two reads
      const missing = state.jobs.filter(j => isFinished(j.status) && !j.result && state.labels[j.id]);
      await Promise.all(missing.map(j => getJob(j.id).then(merge, () => {})));
    } catch (problem) {
      set({ error: (problem instanceof ApiError && problem.message) || '读取任务列表失败' });
    } finally {
      reading = null;
    }
    follow();
  })();
  return reading;
}

function follow() {
  clearTimeout(timer);
  if (!users) { closeStream(); return; }
  const running = state.jobs.find(j => j.status === 'running');
  if (stream && stream.id !== running?.id) closeStream();
  if (running && !stream && typeof EventSource !== 'undefined') openStream(running.id);
  if (state.jobs.some(j => !isFinished(j.status))) timer = setTimeout(() => { void refreshJobs(); }, POLL_MS);
}

function openStream(id: string) {
  const source = new EventSource(jobEventsUrl(id));
  stream = { id, source };
  source.addEventListener('progress', event => {
    const job = state.jobs.find(j => j.id === id);
    try { if (job) merge({ ...job, status: 'running', progress: JSON.parse((event as MessageEvent).data) }); } catch { /* not JSON */ }
  });
  source.addEventListener('result', event => {
    try { merge(JSON.parse((event as MessageEvent).data) as Job); } catch { /* not JSON */ }
    closeStream();
    void refreshJobs();
  });
  // the list, read every few seconds, still says how it ends
  source.onerror = () => { if (stream?.source === source) closeStream(); };
}

function closeStream() {
  stream?.source.close();
  stream = null;
}

/** Starts `method path` as a job named `label`; with `file`, its file can be saved once it has succeeded. */
export async function runJob(label: string, method: 'POST' | 'GET', path: string, params: Params, file = true): Promise<Job> {
  const job = await startJob(method, path, params, file);
  set({ labels: { ...state.labels, [job.id]: label } });
  merge(job);
  follow();
  return job;
}

/** Cancels a job that has not finished, or forgets a finished one. */
export async function removeJob(id: string): Promise<void> {
  const job = state.jobs.find(j => j.id === id);
  try {
    const ended = await dropJob(id);
    if (job && isFinished(job.status)) {
      const { [id]: _, ...labels } = state.labels;
      set({ jobs: state.jobs.filter(j => j.id !== id), labels });
    } else merge(ended);
  } catch (problem) {
    // already forgotten by the server
    if (problem instanceof ApiError && problem.status === 404) set({ jobs: state.jobs.filter(j => j.id !== id) });
    else throw problem;
  }
  void refreshJobs();
}

const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
const snapshot = () => state;

/** The jobs, read while `enabled`; another server or account (`scope`) starts from an empty list. */
export function useJobs(enabled: boolean, scope: number): JobsState {
  useEffect(() => {
    if (!enabled) return;
    users += 1;
    set({ jobs: [], error: undefined });
    void refreshJobs();
    return () => { users -= 1; follow(); };
  }, [enabled, scope]);
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/** One job of the list, or undefined. */
export const findJob = (jobs: JobsState, id: string | null | undefined) => (id ? jobs.jobs.find(j => j.id === id) : undefined);
