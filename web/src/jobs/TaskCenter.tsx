// 任务中心（#211）：`weflow serve` 的全部作业，最新的在前，显示进度、成功后可以下载的文件，
// 以及取消作业或把它从列表里删掉的按钮。
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

/** 作业列表，最新的在前，每个作业显示状态和可以做的操作；没有作业时显示 `empty`。 */
export function TaskCenter({ jobs, names, empty = t('还没有任务。导出等耗时的命令会在这里排队、显示进度。') }: {
  jobs: JobsState;
  /** 会话 ID 到名称的映射，用来给别处启动的导出作业写上它导出的会话的名称。 */
  names?: ReadonlyMap<string, string>;
  empty?: string;
}) {
  if (jobs.error && !jobs.jobs.length) return <p className="empty-text" role="alert">{jobs.error}</p>;
  if (!jobs.jobs.length) return <p className="empty-text">{empty}</p>;
  return <ol className="task-list jobs" aria-live="polite">
    {jobs.jobs.map(job => <JobRow key={job.id} job={job} label={jobs.labels[job.id] ?? describeCommand(job.command, names)} />)}
  </ol>;
}

/** 卡片标题栏上的摘要：运行中和排队的作业数；都没有时是作业的总数或「没有任务」。 */
export function jobsSummary(jobs: JobsState): string {
  const running = jobs.jobs.filter(j => j.status === 'running').length;
  const queued = jobs.jobs.filter(j => j.status === 'queued').length;
  if (running || queued) return [running && t('{n} 个运行中', { n: running }), queued && t('{n} 个排队', { n: queued })].filter(Boolean).join(' · ');
  return jobs.jobs.length ? t('{n} 个任务', { n: jobs.jobs.length }) : t('没有任务');
}
