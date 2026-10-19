// The setup of an account (#216), in four steps: the WeChat data directory and its account, the database key, the
// image keys, and a test that the database opens. Each step runs the CLI command behind it on the server and shows
// its reason when it fails. The keys are fetched with `--save`, so the page only learns that they are set.
import { useEffect, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { ArrowClockwise, Check, CircleNotch, FolderOpen, Key, X } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { isFinished, jobEventsUrl } from '../api/jobs';
import {
  detectDirs, findAccounts, isDatabaseKey, saveImageKeys, serverPlatform, setConfig, testDatabase,
  type Account, type Profile,
} from '../api/setup';
import { findJob, removeJob, runJob, type JobsState } from '../jobs/useJobs';

export const STEPS = [
  { id: 'dir', label: '数据库目录' }, { id: 'key', label: '数据库密钥' },
  { id: 'image', label: '图片密钥' }, { id: 'test', label: '验证' },
] as const;
type Step = typeof STEPS[number]['id'];

const reason = (problem: unknown, fallback: string) =>
  problem instanceof ApiError && problem.message ? problem.message : fallback;

/** One choice of a list: inset when chosen, like the other choices of the page. */
function Choice({ on, onClick, children }: { on: boolean; onClick(): void; children: ReactNode }) {
  return <button type="button" className="setup-choice" aria-pressed={on} onClick={onClick}>
    <span className="setup-choice-copy">{children}</span>{on && <Check size={14} weight="bold" aria-hidden="true" />}
  </button>;
}

function Note({ error, children }: { error?: boolean; children: ReactNode }) {
  return <p className={'setup-note' + (error ? ' error' : '')} role={error ? 'alert' : 'status'}>{children}</p>;
}

/** Step 1: `db detect` and `db wxid`, then `config set db_path` and `config set wxid`. */
function DirStep({ profile, onSaved }: { profile: Profile | undefined; onSaved(): void }) {
  const [dirs, setDirs] = useState<string[] | null>(null);
  const [root, setRoot] = useState(profile?.dbPath ?? '');
  const [typed, setTyped] = useState('');
  const [accounts, setAccounts] = useState<Account[] | null>(null);
  const [wxid, setWxid] = useState(profile?.wxid ?? '');
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const controller = new AbortController();
    detectDirs(controller.signal).then(found => {
      setDirs(found);
      setRoot(old => old || (found.length === 1 ? found[0] : ''));
    }, problem => { if (!controller.signal.aborted) { setDirs([]); setNote({ text: reason(problem, '查找微信数据目录失败。'), error: true }); } });
    return () => controller.abort();
  }, []);

  useEffect(() => {
    setAccounts(null);
    if (!root) return;
    const controller = new AbortController();
    findAccounts(root, controller.signal).then(found => {
      setAccounts(found);
      setWxid(old => (found.some(a => a.wxid === old) ? old : found.length === 1 ? found[0].wxid : ''));
      setNote(null);
    }, problem => {
      if (controller.signal.aborted) return;
      setAccounts([]);
      setNote({ text: reason(problem, '这个文件夹里没有找到微信账号。') + ' 换一个文件夹试试，它应当是微信的数据目录（如 xwechat_files）。', error: true });
    });
    return () => controller.abort();
  }, [root]);

  const useTyped = (event: FormEvent) => { event.preventDefault(); if (typed.trim()) setRoot(typed.trim()); };
  const save = async () => {
    setBusy(true);
    try {
      await setConfig('db_path', root);
      await setConfig('wxid', wxid);
      onSaved();
    } catch (problem) {
      setNote({ text: reason(problem, '保存失败，请重试。'), error: true });
    } finally {
      setBusy(false);
    }
  };
  const shownDirs = dirs ? [...new Set([...dirs, ...(root ? [root] : [])])] : [];

  return <>
    <p className="setup-lead">选择微信的数据目录和其中的账号。下面列出的是在运行 weflow serve 的电脑上找到的目录。</p>
    {dirs === null
      ? <p className="setup-note" aria-busy="true"><CircleNotch size={14} className="spin" /> 正在查找微信数据目录…</p>
      : <div className="setup-choices" role="group" aria-label="微信数据目录">
        {shownDirs.map(dir => <Choice key={dir} on={root === dir} onClick={() => setRoot(dir)}><span className="setup-path">{dir}</span></Choice>)}
        {!shownDirs.length && <p className="setup-note">没有在常用位置找到微信数据目录，请在下面填写。</p>}
      </div>}
    <form className="token-form" onSubmit={useTyped}>
      <label className="token-input"><FolderOpen size={16} aria-hidden="true" />
        <input type="text" spellCheck={false} aria-label="其他文件夹" placeholder="其他文件夹的完整路径" value={typed} onChange={event => setTyped(event.target.value)} />
      </label>
      <button type="submit" className="secondary-button" disabled={!typed.trim()}>查找账号</button>
    </form>
    {root && accounts === null && <p className="setup-note" aria-busy="true"><CircleNotch size={14} className="spin" /> 正在读取账号…</p>}
    {accounts && accounts.length > 0 && <div className="setup-choices" role="group" aria-label="账号">
      {accounts.map(a => <Choice key={a.wxid + a.path} on={wxid === a.wxid} onClick={() => setWxid(a.wxid)}><strong>{a.wxid}</strong><span className="setup-path">{a.path}</span></Choice>)}
    </div>}
    {note && <Note error={note.error}>{note.text}</Note>}
    <div className="setup-actions">
      <button type="button" className="primary-button" disabled={!root || !wxid || busy} onClick={save}>{busy ? '正在保存…' : '保存并继续'}</button>
    </div>
  </>;
}

interface KeyPhase { phase?: string; wasRunning?: boolean }

/** The `key_phase` events of a `key db` job: what the person has to do in WeChat now. */
function useKeyPhase(id: string | null): KeyPhase | null {
  const [phase, setPhase] = useState<KeyPhase | null>(null);
  useEffect(() => {
    setPhase(null);
    if (!id || typeof EventSource === 'undefined') return;
    const source = new EventSource(jobEventsUrl(id));
    source.addEventListener('key_phase', event => { try { setPhase(JSON.parse((event as MessageEvent).data) as KeyPhase); } catch { /* not JSON */ } });
    source.addEventListener('result', () => source.close());
    source.onerror = () => source.close();
    return () => source.close();
  }, [id]);
  return phase;
}

function phaseText(phase: KeyPhase | null): string {
  switch (phase?.phase) {
    case 'quit_wechat': return '检测到微信正在运行。请先完全退出微信：托盘图标 → 右键 → 退出微信。';
    case 'open_wechat': return phase.wasRunning ? '微信已退出，请重新打开微信。' : '没有检测到微信，请打开微信。';
    case 'login': return '请在微信的登录窗口点击「进入微信」。';
    default: return '正在准备…';
  }
}

/** Step 2: `key db --save` as a job, or a key typed in and saved with `config set decrypt_key`. */
function KeyStep({ profile, jobs, onSaved, onNext }: { profile: Profile | undefined; jobs: JobsState; onSaved(): void; onNext(): void }) {
  const [jobId, setJobId] = useState<string | null>(null);
  const [typed, setTyped] = useState('');
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const job = findJob(jobs, jobId);
  const phase = useKeyPhase(job && !isFinished(job.status) ? job.id : null);
  const running = Boolean(job && !isFinished(job.status));

  useEffect(() => {
    if (!job || !isFinished(job.status)) return;
    if (job.status === 'succeeded') { setNote({ text: '已获取数据库密钥，保存在服务端的当前档案里。' }); onSaved(); }
    else if (job.status === 'failed') setNote({ text: (job.result?.error?.message || '获取密钥失败。') + ' 可以重试，或者手动填写密钥。', error: true });
    else setNote({ text: '已取消。' });
    setJobId(null);
  }, [job?.status]);

  const start = async () => {
    setNote(null);
    try { setJobId((await runJob('获取数据库密钥', 'POST', 'key/db', { save: true }, false)).id); }
    catch (problem) { setNote({ text: reason(problem, '没有开始，请重试。'), error: true }); }
  };
  const cancel = () => { if (jobId) removeJob(jobId).catch(() => setNote({ text: '取消失败，请重试。', error: true })); };
  const saveTyped = async (event: FormEvent) => {
    event.preventDefault();
    try {
      await setConfig('decrypt_key', typed.trim());
      setTyped('');
      setNote({ text: '已保存数据库密钥。' });
      onSaved();
    } catch (problem) {
      setNote({ text: reason(problem, '保存失败，请重试。'), error: true });
    }
  };

  return <>
    <p className="setup-lead">数据库密钥在微信打开数据库时产生。获取时会请你退出并重新打开微信，再在登录窗口点击「进入微信」，最多等待 180 秒。在 Windows 上，weflow serve 需要以管理员身份运行。</p>
    <p className="setup-state"><span className={'status-dot' + (profile?.decryptKey ? ' done' : '')} />{profile?.decryptKey ? '当前档案已有数据库密钥。' : '当前档案还没有数据库密钥。'}</p>
    {running
      ? <div className="setup-running" aria-live="polite">
        <p><CircleNotch size={14} className="spin" /> {job!.status === 'queued' ? '排队中：前面的任务完成后开始。' : phaseText(phase)}</p>
        <button type="button" className="secondary-button" onClick={cancel}>取消</button>
      </div>
      : <div className="setup-actions start">
        <button type="button" className="primary-button" onClick={start}><Key size={16} />{profile?.decryptKey ? '重新获取' : '获取数据库密钥'}</button>
      </div>}
    {note && <Note error={note.error}>{note.text}</Note>}
    <details className="setup-manual">
      <summary>手动填写密钥</summary>
      <form className="token-form" onSubmit={saveTyped}>
        <label className="token-input"><Key size={16} aria-hidden="true" />
          <input type="password" autoComplete="off" spellCheck={false} aria-label="数据库密钥" placeholder="64 位十六进制" value={typed} onChange={event => setTyped(event.target.value)} />
        </label>
        <button type="submit" className="secondary-button" disabled={!isDatabaseKey(typed)}>保存</button>
      </form>
    </details>
    <div className="setup-actions">
      <button type="button" className="primary-button" disabled={!profile?.decryptKey || running} onClick={onNext}>下一步</button>
    </div>
  </>;
}

/** Step 3: `key image --save` on Windows; elsewhere the page cannot get them yet, and the step can be skipped. */
function ImageStep({ profile, onSaved, onNext }: { profile: Profile | undefined; onSaved(): void; onNext(): void }) {
  const [platform, setPlatform] = useState<Awaited<ReturnType<typeof serverPlatform>> | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    serverPlatform(controller.signal).then(setPlatform, () => { if (!controller.signal.aborted) setPlatform('other'); });
    return () => controller.abort();
  }, []);
  const fetchKeys = async () => {
    setBusy(true); setNote(null);
    try {
      const saved = await saveImageKeys();
      setNote({ text: saved.verified === false ? '已获取图片密钥（只验证了 AES，XOR 尚未验证）。' : '已获取图片密钥，并用本机的图片验证过。' });
      onSaved();
    } catch (problem) {
      setNote({ text: reason(problem, '获取图片密钥失败。') + ' 先在微信里打开几张图片，再重试。', error: true });
    } finally {
      setBusy(false);
    }
  };
  return <>
    <p className="setup-lead">图片密钥用来解密聊天里的图片。没有它也能浏览文字记录，图片会显示为无法解密。</p>
    <p className="setup-state"><span className={'status-dot' + (profile?.imageKeys ? ' done' : '')} />{profile?.imageKeys ? '当前档案已有图片密钥。' : '当前档案还没有图片密钥。'}</p>
    {platform === null
      ? <p className="setup-note" aria-busy="true"><CircleNotch size={14} className="spin" /> 正在检查…</p>
      : platform === 'windows'
        ? <div className="setup-actions start"><button type="button" className="primary-button" disabled={busy} onClick={fetchKeys}><Key size={16} />{busy ? '正在获取…' : profile?.imageKeys ? '重新获取' : '获取图片密钥'}</button></div>
        : <p className="setup-note">{platform === 'macos' ? '网页还不能在 macOS 上获取图片密钥：请在终端运行 weflow key scan-image，或者先跳过。' : '这个平台还不能获取图片密钥，可以先跳过。'}</p>}
    {note && <Note error={note.error}>{note.text}</Note>}
    <div className="setup-actions">
      <button type="button" className={profile?.imageKeys ? 'primary-button' : 'secondary-button'} disabled={busy} onClick={onNext}>{profile?.imageKeys ? '下一步' : '跳过'}</button>
    </div>
  </>;
}

/** Why `db test` failed, in the page's words, and the step that sets it right. */
function testFailure(problem: unknown): { text: string; back: Step } {
  const message = reason(problem, '打不开数据库。');
  if (problem instanceof ApiError && problem.code === 'config_error') {
    if (/db_path|wxid/.test(message)) return { text: '还没有设置数据库目录。', back: 'dir' };
    if (/decrypt_key/.test(message)) return { text: '还没有数据库密钥。', back: 'key' };
    return { text: message, back: 'dir' };
  }
  return { text: message + ' 密钥可能与这个账号不符：重新获取数据库密钥，或者换一个账号。', back: 'key' };
}

/** Step 4: `db test`. */
function TestStep({ onDone, onBack }: { onDone(): void; onBack(step: Step): void }) {
  const [result, setResult] = useState<{ ok: boolean; text: string; back?: Step } | null>(null);
  const [round, setRound] = useState(0);
  useEffect(() => {
    setResult(null);
    const controller = new AbortController();
    testDatabase(controller.signal).then(() => setResult({ ok: true, text: '可以打开数据库，配置完成。' }), problem => {
      if (!controller.signal.aborted) setResult({ ok: false, ...testFailure(problem) });
    });
    return () => controller.abort();
  }, [round]);
  return <>
    <p className="setup-lead">用当前档案的目录和密钥打开数据库。</p>
    {result === null
      ? <p className="setup-note" aria-busy="true"><CircleNotch size={14} className="spin" /> 正在打开数据库…</p>
      : <Note error={!result.ok}>{result.text}</Note>}
    <div className="setup-actions">
      {result?.ok
        ? <button type="button" className="primary-button" onClick={onDone}>开始使用</button>
        : result && <><button type="button" className="secondary-button" onClick={() => onBack(result.back ?? 'key')}>回到{STEPS.find(s => s.id === result.back)?.label}</button>
          <button type="button" className="primary-button" onClick={() => setRound(n => n + 1)}><ArrowClockwise size={16} />重试</button></>}
    </div>
  </>;
}

export function SetupWizard({ profile, jobs, onChanged, onDone, onClose }: {
  /** The current profile as `config profiles` last said, or undefined while it is read. */
  profile: Profile | undefined;
  jobs: JobsState;
  /** A setting was saved: read the profiles again. */
  onChanged(): void;
  onDone(): void;
  onClose(): void;
}) {
  const [step, setStep] = useState<Step>(profile?.dbPath && profile.wxid ? (profile.decryptKey ? 'image' : 'key') : 'dir');
  const index = STEPS.findIndex(s => s.id === step);
  const next = () => setStep(STEPS[Math.min(index + 1, STEPS.length - 1)].id);
  // a step is ticked when the profile has what it sets, wherever the person is
  const done: Record<Step, boolean> = {
    dir: Boolean(profile?.dbPath && profile.wxid), key: Boolean(profile?.decryptKey), image: Boolean(profile?.imageKeys), test: false,
  };
  return <div className="setup-content">
    <header><h2>账号配置{profile ? <small>档案 {profile.name}</small> : null}</h2>
      <button type="button" className="icon-button" aria-label="关闭配置向导" onClick={onClose}><X size={18} /></button></header>
    <ol className="setup-steps">
      {STEPS.map((s, i) => <li key={s.id}>
        <button type="button" className={'setup-step' + (done[s.id] ? ' passed' : '')} aria-current={s.id === step ? 'step' : undefined} onClick={() => setStep(s.id)}>
          <span className="setup-step-number">{done[s.id] ? <Check size={12} weight="bold" aria-label="已完成" /> : i + 1}</span>{s.label}
        </button>
      </li>)}
    </ol>
    <div className="setup-body" key={step}>
      {step === 'dir' && <DirStep profile={profile} onSaved={() => { onChanged(); next(); }} />}
      {step === 'key' && <KeyStep profile={profile} jobs={jobs} onSaved={onChanged} onNext={next} />}
      {step === 'image' && <ImageStep profile={profile} onSaved={onChanged} onNext={next} />}
      {step === 'test' && <TestStep onDone={onDone} onBack={setStep} />}
    </div>
  </div>;
}
