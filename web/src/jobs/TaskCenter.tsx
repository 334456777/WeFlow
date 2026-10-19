// The task centre (#211): every job of `weflow serve`, newest first, with its progress, the file to save once it
// has succeeded, and a way to cancel it or take it off the list.
import { useState } from 'react';
import { Check, Circle, CircleNotch, DownloadSimple, X } from '@phosphor-icons/react';
import { describeCommand, isFinished, jobFileUrl, type Job } from '../api/jobs';
import { formatClock } from '../chat/messages';
import { removeJob, type JobsState } from './useJobs';

const STATUS: Record<Job['status'], string> = { queued: '排队中', running: '正在运行', succeeded: '已完成', failed: '未完成', cancelled: '已取消' };

export const formatSize = (bytes: number) =>
  bytes < 1024 ? bytes + ' B' : bytes < 1024 * 1024 ? (bytes / 1024).toFixed(1) + ' KB' : (bytes / 1024 / 1024).toFixed(1) + ' MB';

/** What the line under a job's name says: its progress, its error, or its file. */
export function jobDetail(job: Job): string {
  if (job.status === 'running') {
    const p = job.progress;
    const count = p?.total ? (p.current ?? 0) + ' / ' + p.total : '';
    return [p?.message, count, p?.percent !== undefined ? Math.round(p.percent) + '%' : ''].filter(Boolean).join(' · ') || '正在运行';
  }
  if (job.status === 'failed') return job.result?.error?.message || '命令没有完成' + (job.httpStatus ? '（HTTP ' + job.httpStatus + '）' : '');
  if (job.status === 'succeeded' && job.file) return job.file.name + (job.file.size !== null ? ' · ' + formatSize(job.file.size) : '');
  return STATUS[job.status];
}

/** The folder of a finished export on the server, when the result names its file. */
export function outputOf(job: Job): string {
  const out = job.result?.data?.out;
  return typeof out === 'string' ? out : '';
}

function JobRow({ job, label }: { job: Job; label: string }) {
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState('');
  const done = isFinished(job.status);
  const remove = () => {
    setBusy(true); setProblem('');
    removeJob(job.id).catch(() => setProblem(done ? '移除失败，请重试。' : '取消失败，请重试。')).finally(() => setBusy(false));
  };
  const percent = job.status === 'running' && job.progress?.percent !== undefined ? Math.max(0, Math.min(100, job.progress.percent)) : null;
  return <li className={'task job ' + (job.status === 'succeeded' ? 'complete' : job.status === 'failed' ? 'error' : '')}>
    <span className="task-icon">{job.status === 'running' ? <CircleNotch size={16} className="spin" /> : job.status === 'succeeded' ? <Check size={12} weight="bold" /> : job.status === 'failed' ? <X size={12} weight="bold" /> : <Circle size={16} />}</span>
    <div>
      <strong className="clamp-1">{label}</strong>
      <small className="clamp-2">{STATUS[job.status] === jobDetail(job) ? STATUS[job.status] : STATUS[job.status] + ' · ' + jobDetail(job)}</small>
      {percent !== null && <span className="job-bar" role="progressbar" aria-label={label + '的进度'} aria-valuenow={Math.round(percent)} aria-valuemin={0} aria-valuemax={100}><span style={{ width: percent + '%' }} /></span>}
      {problem && <small role="alert">{problem}</small>}
      <span className="job-actions">
        {job.status === 'succeeded' && job.file && <a className="download-link" href={jobFileUrl(job.id)} download={job.file.name}><DownloadSimple size={12} />下载文件</a>}
        <button type="button" className="text-button" disabled={busy} onClick={remove}>{done ? '移除' : '取消'}</button>
      </span>
    </div>
    <time className="num">{formatClock(Math.floor(job.createdAt / 1000))}</time>
  </li>;
}

export function TaskCenter({ jobs, empty = '还没有任务。导出等耗时的命令会在这里排队、显示进度。' }: { jobs: JobsState; empty?: string }) {
  if (jobs.error && !jobs.jobs.length) return <p className="empty-text" role="alert">{jobs.error}</p>;
  if (!jobs.jobs.length) return <p className="empty-text">{empty}</p>;
  return <ol className="task-list jobs" aria-live="polite">
    {jobs.jobs.map(job => <JobRow key={job.id} job={job} label={jobs.labels[job.id] ?? describeCommand(job.command)} />)}
  </ol>;
}

/** The card's summary: what runs or waits. */
export function jobsSummary(jobs: JobsState): string {
  const running = jobs.jobs.filter(j => j.status === 'running').length;
  const queued = jobs.jobs.filter(j => j.status === 'queued').length;
  if (running || queued) return [running && running + ' 个运行中', queued && queued + ' 个排队'].filter(Boolean).join(' · ');
  return jobs.jobs.length ? jobs.jobs.length + ' 个任务' : '没有任务';
}
