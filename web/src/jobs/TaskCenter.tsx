// The task centre (#211): every job of `weflow serve`, newest first, with its progress, the file to save once it
// has succeeded, and a way to cancel it or take it off the list.
import { useState } from 'react';
import { Check, Circle, CircleNotch, DownloadSimple, X } from '@phosphor-icons/react';
import { describeCommand, isFinished, jobFileUrl, jobLine, type Job } from '../api/jobs';
import { formatClock } from '../format';
import { FormatLanes } from '../exports/FormatLanes';
import { removeJob, type JobsState } from './useJobs';
import { t } from '../i18n';

function JobRow({ job, label }: { job: Job; label: string }) {
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState('');
  const done = isFinished(job.status);
  const remove = () => {
    setBusy(true); setProblem('');
    removeJob(job.id).catch(() => setProblem(done ? t('删除失败，请重试。') : t('取消失败，请重试。'))).finally(() => setBusy(false));
  };
  const percent = job.status === 'running' && job.progress?.percent !== undefined ? Math.max(0, Math.min(100, job.progress.percent)) : null;
  return <li className={'task job ' + (job.status === 'succeeded' ? 'complete' : job.status === 'failed' ? 'error' : '')}>
    <span className="task-icon">{job.status === 'running' ? <CircleNotch size={16} className="spin" /> : job.status === 'succeeded' ? <Check size={12} weight="bold" /> : job.status === 'failed' ? <X size={12} weight="bold" /> : <Circle size={16} />}</span>
    <div>
      <strong className="clamp-1">{label}</strong>
      <small className="clamp-2">{jobLine(job)}</small>
      {job.status === 'running' && <FormatLanes progress={job.progress} />}
      {percent !== null && !job.progress?.formats && <span className="job-bar" role="progressbar" aria-label={t('{label}的进度', { label })} aria-valuenow={Math.round(percent)} aria-valuemin={0} aria-valuemax={100}><span style={{ width: percent + '%' }} /></span>}
      {problem && <small role="alert">{problem}</small>}
      <span className="job-actions">
        {job.status === 'succeeded' && job.file && <a className="download-link" href={jobFileUrl(job.id)} download={job.file.name ?? undefined}><DownloadSimple size={12} />{t('下载文件')}</a>}
        <button type="button" className="text-button" disabled={busy} onClick={remove} title={done ? (job.file ? t('删除这个任务和它在服务端的文件') : t('删除这个任务')) : t('取消这个任务')}>{done ? t('删除') : t('取消')}</button>
      </span>
    </div>
    <time className="num">{formatClock(Math.floor(job.createdAt / 1000))}</time>
  </li>;
}

/** The jobs, newest first, each with where it stands and what can be done with it; `empty` without any. */
export function TaskCenter({ jobs, names, empty = t('还没有任务。导出等耗时的命令会在这里排队、显示进度。') }: {
  jobs: JobsState;
  /** The conversations by id, to name the one a job started elsewhere exports. */
  names?: ReadonlyMap<string, string>;
  empty?: string;
}) {
  if (jobs.error && !jobs.jobs.length) return <p className="empty-text" role="alert">{jobs.error}</p>;
  if (!jobs.jobs.length) return <p className="empty-text">{empty}</p>;
  return <ol className="task-list jobs" aria-live="polite">
    {jobs.jobs.map(job => <JobRow key={job.id} job={job} label={jobs.labels[job.id] ?? describeCommand(job.command, names)} />)}
  </ol>;
}

/** The card's summary: what runs or waits. */
export function jobsSummary(jobs: JobsState): string {
  const running = jobs.jobs.filter(j => j.status === 'running').length;
  const queued = jobs.jobs.filter(j => j.status === 'queued').length;
  if (running || queued) return [running && t('{n} 个运行中', { n: running }), queued && t('{n} 个排队', { n: queued })].filter(Boolean).join(' · ');
  return jobs.jobs.length ? t('{n} 个任务', { n: jobs.jobs.length }) : t('没有任务');
}
