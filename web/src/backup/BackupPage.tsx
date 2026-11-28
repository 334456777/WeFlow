// 数据库备份页（#212）：作为服务端的作业创建备份并从浏览器下载，列出这个标签页创建的备份，
// 以及查看服务端电脑上的备份里有什么、把它恢复到那台电脑上的某个文件夹。
import { useContext, useState } from 'react';
import type { FormEvent } from 'react';
import { Archive, ArrowCounterClockwise, Check, CircleNotch, DownloadSimple, FolderOpen, MagnifyingGlass } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { backupGaps, backupParams, backupParts, inspectBackup, type BackupChoice, type BackupGaps, type BackupManifest } from '../api/backup';
import { jobFileUrl, jobLine, type Job } from '../api/jobs';
import { formatSize } from '../format';
import { runJob, type JobsState } from '../jobs/useJobs';
import type { Page, Shell } from '../shell/page';
import { Connected, ScrollPage } from '../shell/panels';
import { SampleContext } from '../shell/sample';
import { locale, t } from '../i18n';
import './BackupPage.css';

const reason = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || (problem instanceof Error && problem.message) || fallback;
type Note = { text: string; error?: boolean } | null;
const NoteLine = ({ note }: { note: Note }) => note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>;

function Check3({ label, hint, checked, onChange }: { label: string; hint: string; checked: boolean; onChange(on: boolean): void }) {
  return <label className="checkbox"><input type="checkbox" aria-label={label} checked={checked} onChange={event => onChange(event.target.checked)} />
    <span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>{label} <small>{hint}</small></span></label>;
}

/** 备份缺少的内容（`backupGaps`），每项一行警告；什么都不缺时不显示。 */
const Gaps = ({ lines }: { lines: string[] }) => <>{lines.map(line => <p key={line} className="card-note setting-warning backup-gap">{line}</p>)}</>;

/** 作业创建的备份在服务端的路径，取自作业的结果；结果里没有时为 null。 */
const backupPath = (job: Job) => {
  const out = job.result?.data?.out;
  return typeof out === 'string' ? out : null;
};

/** 备份页的内容：创建备份、这个标签页创建的备份，以及查看或恢复一个备份。 */
export function BackupPage({ jobs }: { jobs: JobsState }) {
  const [parts, setParts] = useState<BackupChoice>({ images: false, videos: false, files: false, emojis: false });
  const [note, setNote] = useState<Note>(null);
  // 示例数据里的备份（#879），可以像服务端电脑上的备份一样查看
  const sample = useContext(SampleContext);
  const [path, setPath] = useState(sample ? 'weflow-backup.zip' : '');
  const [target, setTarget] = useState('');
  const [manifest, setManifest] = useState<{ path: string; value: BackupManifest } | null>(null);
  const [busy, setBusy] = useState<'create' | 'inspect' | 'restore' | null>(null);
  const [lookNote, setLookNote] = useState<Note>(null);
  const made = jobs.jobs.filter(j => j.command.includes('backup') && j.command.includes('create') && jobs.labels[j.id]);

  const create = async () => {
    setBusy('create'); setNote(null);
    try {
      await runJob(t('备份数据库'), 'POST', 'backup/create', backupParams(parts), true);
      setNote({ text: t('已开始备份，进度在任务中心和下面的列表里。') });
    } catch (problem) {
      setNote({ text: reason(problem, t('没有开始，请重试。')), error: true });
    } finally {
      setBusy(null);
    }
  };
  const inspect = async (where: string) => {
    const at = where.trim();
    if (!at) return;
    setPath(at); setBusy('inspect'); setLookNote(null); setManifest(null);
    try { setManifest({ path: at, value: await inspectBackup(at) }); }
    catch (problem) { setLookNote({ text: reason(problem, t('读不了这个备份。')), error: true }); }
    finally { setBusy(null); }
  };
  const restore = async (event: FormEvent) => {
    event.preventDefault();
    if (!manifest || !target.trim()) return;
    setBusy('restore'); setLookNote(null);
    try {
      await runJob(t('恢复备份'), 'POST', 'backup/restore', { path: manifest.path, out: target.trim() }, false);
      setLookNote({ text: t('已开始恢复：先完整校验备份，校验通过才写入。进度在任务中心。') });
    } catch (problem) {
      setLookNote({ text: reason(problem, t('没有开始，请重试。')), error: true });
    } finally {
      setBusy(null);
    }
  };

  return <div className="backup-page">
    <section className="backup-block" aria-label={t('创建备份')}>
      <h2><Archive size={18} />{t('创建备份')}</h2>
      <p className="muted">{t('把当前账号的数据库打包成一个 zip，在服务端生成，完成后从浏览器下载。')}</p>
      <p className="muted">{t('数据库里已经有聊天记录和语音。图片、视频和文件可能很大，需要时再勾选。')}</p>
      <div className="backup-parts">
        <Check3 label={t('图片')} hint={t('微信保存的聊天图片（msg/attach）')} checked={parts.images} onChange={images => setParts(p => ({ ...p, images }))} />
        <Check3 label={t('视频')} hint={t('微信保存的视频（msg/video）')} checked={parts.videos} onChange={videos => setParts(p => ({ ...p, videos }))} />
        <Check3 label={t('文件')} hint={t('收发的文件（msg/file）')} checked={parts.files} onChange={files => setParts(p => ({ ...p, files }))} />
        <Check3 label={t('表情')} hint={t('WeFlow 下载的表情，恢复时放回表情缓存')} checked={parts.emojis} onChange={emojis => setParts(p => ({ ...p, emojis }))} />
      </div>
      <button type="button" className="primary-button large export-pending" disabled={busy === 'create'} onClick={create}>{busy === 'create' ? <CircleNotch size={18} className="spin" /> : <Archive size={18} />}{t('创建备份')}</button>
      <NoteLine note={note} />
      {made.length > 0 && <ul className="backup-made" aria-label={t('这个标签页创建的备份')}>
        {made.map(job => {
          const done = job.status === 'succeeded';
          const where = backupPath(job);
          return <li key={job.id}>
            <span className="account-copy"><strong>{new Date(job.createdAt).toLocaleString(locale(), { hour12: false })}</strong>
              <span className={'cache-text' + (job.status === 'failed' ? ' backup-failed' : '')}>{jobLine(job)}</span></span>
            {done && job.file && <a className="secondary-button" href={jobFileUrl(job.id)} download={job.file.name}><DownloadSimple size={12} />{t('下载')}</a>}
            {done && where && <button type="button" className="secondary-button" onClick={() => inspect(where)}><MagnifyingGlass size={12} />{t('查看内容')}</button>}
            {done && <Gaps lines={backupGaps((job.result?.data ?? {}) as BackupGaps)} />}
          </li>;
        })}
      </ul>}
    </section>

    <section className="backup-block" aria-label={t('查看与恢复')}>
      <h2><ArrowCounterClockwise size={18} />{t('查看与恢复')}</h2>
      <p className="muted">{t('备份在运行 weflow serve 的电脑上：填它的完整路径，或者在上面点「查看内容」。网页还不能上传备份包。')}</p>
      <form className="token-form" onSubmit={event => { event.preventDefault(); void inspect(path); }}>
        <label className="token-input"><Archive size={16} aria-hidden="true" />
          <input type="text" spellCheck={false} aria-label={t('备份的路径')} placeholder={t('备份 zip 在服务端的完整路径')} value={path} onChange={event => setPath(event.target.value)} /></label>
        <button type="submit" className="secondary-button" disabled={!path.trim() || busy === 'inspect'}>{busy === 'inspect' ? <CircleNotch size={12} className="spin" /> : null}{t('查看')}</button>
      </form>
      {manifest && <div className="backup-manifest" aria-label={t('备份的内容')}>
        <dl className="info-grid">
          <div><dt>{t('账号')}</dt><dd className="info-id">{manifest.value.wxid || '—'}</dd></div>
          <div><dt>{t('创建于')}</dt><dd className="num">{manifest.value.created_at ? new Date(manifest.value.created_at * 1000).toLocaleString(locale(), { hour12: false }) : '—'}</dd></div>
          <div><dt>{t('WeFlow 版本')}</dt><dd className="num">{manifest.value.weflow_version || '—'}</dd></div>
          {backupParts(manifest.value.entries).map(p => <div key={p.label}><dt>{t(p.label)}</dt><dd className="num">{t('{files} 个 · {bytes}', { files: p.files.toLocaleString(locale()), bytes: formatSize(p.bytes) })}</dd></div>)}
        </dl>
        <Gaps lines={backupGaps(manifest.value)} />
        <form className="token-form" onSubmit={restore}>
          <label className="token-input"><FolderOpen size={16} aria-hidden="true" />
            <input type="text" spellCheck={false} aria-label={t('恢复到的文件夹')} placeholder={t('恢复到服务端的哪个文件夹')} value={target} onChange={event => setTarget(event.target.value)} /></label>
          <button type="submit" className="secondary-button" disabled={!target.trim() || busy === 'restore'}>{busy === 'restore' ? <CircleNotch size={12} className="spin" /> : <ArrowCounterClockwise size={12} />}{t('恢复')}</button>
        </form>
        <p className="card-note">{t('恢复会先校验备份里每个文件的 SHA-256，全部通过才写入这个文件夹。不要选微信正在使用的数据目录。')}</p>
      </div>}
      <NoteLine note={lookNote} />
    </section>
  </div>;
}

/**
 * 工作区里的备份页，旁边的任务中心显示正在运行的备份。换了服务端或账号（即作用域；
 * 连上的服务端取代示例数据时也会换新的作用域）时页面重新开始，因为已填的路径和读到的备份都属于之前的那一个。
 */
export function backupPage(shell: Shell): Page {
  const view = <ScrollPage label={t('数据库备份')} icon={Archive}>
    <Connected backend={shell.backend} offline={t('连接 weflow serve 后，可以在这里备份和恢复数据库。')}><BackupPage key={shell.scope} jobs={shell.jobs} /></Connected>
  </ScrollPage>;
  return { view, shown: ['tasks'] };
}
