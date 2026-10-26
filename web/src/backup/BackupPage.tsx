// The database backup page (#212): a backup made as a job of the server and saved from the browser, the backups this
// tab made, and what a backup on the server's computer holds or restoring it into a folder there.
import { useState } from 'react';
import type { FormEvent } from 'react';
import { Archive, ArrowCounterClockwise, Check, CircleNotch, DownloadSimple, FolderOpen, MagnifyingGlass } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { backupParams, backupParts, inspectBackup, type BackupManifest } from '../api/backup';
import { jobFileUrl, type Job } from '../api/jobs';
import { formatSize, jobLine } from '../jobs/TaskCenter';
import { runJob, type JobsState } from '../jobs/useJobs';
import type { Page, Shell } from '../shell/page';
import { Connected, ScrollPage } from '../shell/panels';

const reason = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || (problem instanceof Error && problem.message) || fallback;
type Note = { text: string; error?: boolean } | null;
const NoteLine = ({ note }: { note: Note }) => note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>;

function Check3({ label, hint, checked, onChange }: { label: string; hint: string; checked: boolean; onChange(on: boolean): void }) {
  return <label className="checkbox"><input type="checkbox" aria-label={label} checked={checked} onChange={event => onChange(event.target.checked)} />
    <span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>{label} <small>{hint}</small></span></label>;
}

/** Where the backup a job made sits on the server, from its result. */
const backupPath = (job: Job) => {
  const out = job.result?.data?.out;
  return typeof out === 'string' ? out : null;
};

export function BackupPage({ jobs }: { jobs: JobsState }) {
  const [parts, setParts] = useState({ images: true, voice: true, emojis: true });
  const [note, setNote] = useState<Note>(null);
  const [path, setPath] = useState('');
  const [target, setTarget] = useState('');
  const [manifest, setManifest] = useState<{ path: string; value: BackupManifest } | null>(null);
  const [busy, setBusy] = useState<'create' | 'inspect' | 'restore' | null>(null);
  const [lookNote, setLookNote] = useState<Note>(null);
  const made = jobs.jobs.filter(j => j.command.includes('backup') && j.command.includes('create') && jobs.labels[j.id]);

  const create = async () => {
    setBusy('create'); setNote(null);
    try {
      await runJob('备份数据库', 'POST', 'backup/create', backupParams(parts), true);
      setNote({ text: '已开始备份，进度在任务中心和下面的列表里。' });
    } catch (problem) {
      setNote({ text: reason(problem, '没有开始，请重试。'), error: true });
    } finally {
      setBusy(null);
    }
  };
  const inspect = async (where: string) => {
    const at = where.trim();
    if (!at) return;
    setPath(at); setBusy('inspect'); setLookNote(null); setManifest(null);
    try { setManifest({ path: at, value: await inspectBackup(at) }); }
    catch (problem) { setLookNote({ text: reason(problem, '读不了这个备份。'), error: true }); }
    finally { setBusy(null); }
  };
  const restore = async (event: FormEvent) => {
    event.preventDefault();
    if (!manifest || !target.trim()) return;
    setBusy('restore'); setLookNote(null);
    try {
      await runJob('恢复备份', 'POST', 'backup/restore', { path: manifest.path, target: target.trim() }, false);
      setLookNote({ text: '已开始恢复：先完整校验备份，校验通过才写入。进度在任务中心。' });
    } catch (problem) {
      setLookNote({ text: reason(problem, '没有开始，请重试。'), error: true });
    } finally {
      setBusy(null);
    }
  };

  return <div className="backup-page">
    <section className="backup-block" aria-label="创建备份">
      <h2><Archive size={18} />创建备份</h2>
      <p className="muted">把当前账号的数据库打包成一个 zip，在服务端生成，完成后从浏览器下载。</p>
      <div className="backup-parts">
        <Check3 label="图片" hint="微信保存的聊天图片（FileStorage/Image）" checked={parts.images} onChange={images => setParts(p => ({ ...p, images }))} />
        <Check3 label="语音" hint="FileStorage/Audio" checked={parts.voice} onChange={voice => setParts(p => ({ ...p, voice }))} />
        <Check3 label="表情" hint="WeFlow 缓存的表情" checked={parts.emojis} onChange={emojis => setParts(p => ({ ...p, emojis }))} />
      </div>
      <button type="button" className="primary-button large export-pending" disabled={busy === 'create'} onClick={create}>{busy === 'create' ? <CircleNotch size={18} className="spin" /> : <Archive size={18} />}创建备份</button>
      <NoteLine note={note} />
      {made.length > 0 && <ul className="backup-made" aria-label="这个标签页创建的备份">
        {made.map(job => {
          const done = job.status === 'succeeded';
          const where = backupPath(job);
          return <li key={job.id}>
            <span className="account-copy"><strong>{new Date(job.createdAt).toLocaleString('zh-CN', { hour12: false })}</strong>
              <span className={'cache-text' + (job.status === 'failed' ? ' backup-failed' : '')}>{jobLine(job)}</span></span>
            {done && job.file && <a className="secondary-button" href={jobFileUrl(job.id)} download={job.file.name}><DownloadSimple size={12} />下载</a>}
            {done && where && <button type="button" className="secondary-button" onClick={() => inspect(where)}><MagnifyingGlass size={12} />查看内容</button>}
          </li>;
        })}
      </ul>}
    </section>

    <section className="backup-block" aria-label="查看与恢复">
      <h2><ArrowCounterClockwise size={18} />查看与恢复</h2>
      <p className="muted">备份在运行 weflow serve 的电脑上：填它的完整路径，或者在上面点「查看内容」。网页还不能上传备份包。</p>
      <form className="token-form" onSubmit={event => { event.preventDefault(); void inspect(path); }}>
        <label className="token-input"><Archive size={16} aria-hidden="true" />
          <input type="text" spellCheck={false} aria-label="备份的路径" placeholder="备份 zip 在服务端的完整路径" value={path} onChange={event => setPath(event.target.value)} /></label>
        <button type="submit" className="secondary-button" disabled={!path.trim() || busy === 'inspect'}>{busy === 'inspect' ? <CircleNotch size={12} className="spin" /> : null}查看</button>
      </form>
      {manifest && <div className="backup-manifest" aria-label="备份的内容">
        <dl className="info-grid">
          <div><dt>账号</dt><dd className="info-id">{manifest.value.wxid || '—'}</dd></div>
          <div><dt>创建于</dt><dd className="num">{manifest.value.created_at ? new Date(manifest.value.created_at * 1000).toLocaleString('zh-CN', { hour12: false }) : '—'}</dd></div>
          <div><dt>WeFlow 版本</dt><dd className="num">{manifest.value.weflow_version || '—'}</dd></div>
          {backupParts(manifest.value.entries).map(p => <div key={p.label}><dt>{p.label}</dt><dd className="num">{p.files.toLocaleString('zh-CN')} 个 · {formatSize(p.bytes)}</dd></div>)}
        </dl>
        <form className="token-form" onSubmit={restore}>
          <label className="token-input"><FolderOpen size={16} aria-hidden="true" />
            <input type="text" spellCheck={false} aria-label="恢复到的文件夹" placeholder="恢复到服务端的哪个文件夹" value={target} onChange={event => setTarget(event.target.value)} /></label>
          <button type="submit" className="secondary-button" disabled={!target.trim() || busy === 'restore'}>{busy === 'restore' ? <CircleNotch size={12} className="spin" /> : <ArrowCounterClockwise size={12} />}恢复</button>
        </form>
        <p className="card-note">恢复会先校验备份里每个文件的 SHA-256，全部通过才写入这个文件夹。不要选微信正在使用的数据目录。</p>
      </div>}
      <NoteLine note={lookNote} />
    </section>
  </div>;
}

/** The page in the workspace: the task centre beside it shows the backup running. */
export function backupPage(shell: Shell): Page {
  const view = <ScrollPage label="数据库备份" icon={Archive}>
    <Connected backend={shell.backend} offline="连接 weflow serve 后，可以在这里备份和恢复数据库。"><BackupPage jobs={shell.jobs} /></Connected>
  </ScrollPage>;
  return { view, shown: ['tasks'] };
}
