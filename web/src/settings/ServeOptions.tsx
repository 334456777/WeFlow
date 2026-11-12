// The options weflow serve starts with (#406), saved in its config file (`serve_*`): each is used when the command
// line does not give it, from the next start of the server.
import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { Check, CircleNotch } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { getSetting, setSetting, unsetSetting } from '../api/settings';
import { GridSkeleton } from '../Skeleton';
import { msg, t } from '../i18n';

const reason = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || fallback;
type Note = { text: string; error?: boolean } | null;

/** The whole numbers, with their labels and what the server uses when none is saved; shown through t(). */
const NUMBERS = [
  { key: 'serve_port', label: msg('端口'), fallback: '18108' },
  { key: 'serve_max_tasks', label: msg('同时运行的任务'), fallback: '1' },
  { key: 'serve_max_reads', label: msg('同时运行的其他命令'), fallback: msg('CPU 核数') },
  { key: 'serve_max_media', label: msg('同时解码的图片和语音'), fallback: msg('CPU 核数') },
] as const;

/** The switches, with what the server does when none is saved; shown through t(). */
const SWITCHES = [
  { key: 'serve_open_browser', label: msg('启动时在浏览器中打开网页'), fallback: true },
  { key: 'serve_insight', label: msg('运行灵感引擎'), fallback: false },
  { key: 'serve_image_auto_download', label: msg('自动下载图片（仅 Windows）'), fallback: true },
  { key: 'serve_log_requests', label: msg('每个请求在终端输出一行'), fallback: false },
] as const;

const KEYS = ['serve_host', ...NUMBERS.map(n => n.key), ...SWITCHES.map(s => s.key)];
/** Each key's saved value as text: `''` when none is saved. */
type Form = Record<string, string>;

const wholeNumber = (text: string) => !text.trim() || (/^\d+$/.test(text.trim()) && Number(text) >= 1 && Number(text) <= 65535);
/** A listen address only this machine reaches. */
const loopback = (host: string) => !host || host === 'localhost' || host.startsWith('127.') || host === '::1' || host === '[::1]';

export function ServeOptions() {
  const [saved, setSaved] = useState<Form | null>(null);
  const [form, setForm] = useState<Form | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<Note>(null);
  useEffect(() => {
    const controller = new AbortController();
    Promise.all(KEYS.map(k => getSetting(k, controller.signal))).then(values => {
      const read = Object.fromEntries(KEYS.map((k, i) => [k, values[i] ?? '']));
      setSaved(read); setForm(read);
    }, problem => { if (!controller.signal.aborted) setNote({ text: reason(problem, t('读不到启动选项。')), error: true }); });
    return () => controller.abort();
  }, []);
  const set = (key: string, value: string) => setForm(old => (old ? { ...old, [key]: value } : old));
  const bad = form ? NUMBERS.filter(n => !wholeNumber(form[n.key])).map(n => n.key) : [];
  const changed = Boolean(form && saved && KEYS.some(k => form[k].trim() !== saved[k]));

  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (!form || !saved || bad.length) return;
    setBusy(true); setNote(null);
    // what was saved before a failure stays saved
    const next = { ...saved };
    try {
      for (const k of KEYS) {
        const value = form[k].trim();
        if (value === saved[k]) continue;
        await (value ? setSetting(k, value) : unsetSetting(k));
        next[k] = value;
      }
      setNote({ text: t('已保存，重启 weflow serve 后生效。') });
    } catch (problem) {
      setNote({ text: reason(problem, t('保存失败，请重试。')), error: true });
    } finally {
      setSaved(next);
      setBusy(false);
    }
  };

  return <div className="setting-row setting-accounts"><div><h2>{t('启动选项')}</h2><p>{t('weflow serve 启动时用的选项，保存在服务端的配置文件里，重启后生效。启动命令里给出的选项优先。')}</p>
    {!form ? (note ? <p className="account-note setting-warning" role="alert">{note.text}</p> : <GridSkeleton label={t('正在读取启动选项')} cells={4} />) : <form className="ai-form serve-form" onSubmit={save}>
      <label className="console-field"><span className="console-field-name">{t('监听地址')}</span>
        <input className="console-input" type="text" spellCheck={false} placeholder="127.0.0.1" value={form.serve_host} onChange={e => set('serve_host', e.target.value)} />
        {!loopback(form.serve_host.trim()) && <small className="setting-warning">{t('监听本机以外的地址时，持有令牌的人可以在这台机器上运行任何 weflow 命令。')}</small>}</label>
      {NUMBERS.map(n => <label key={n.key} className="console-field"><span className="console-field-name">{t(n.label)}</span>
        <input className="console-input" type="text" inputMode="numeric" placeholder={t(n.fallback)} value={form[n.key]} onChange={e => set(n.key, e.target.value)} />
        {bad.includes(n.key) && <small className="setting-warning">{t('要填 1 到 65535 的整数。')}</small>}</label>)}
      {SWITCHES.map(s => {
        const on = form[s.key] ? form[s.key] === 'true' : s.fallback;
        // the server's default is saved as nothing
        return <label key={s.key} className="checkbox"><input type="checkbox" aria-label={t(s.label)} checked={on} onChange={e => set(s.key, e.target.checked === s.fallback ? '' : String(e.target.checked))} />
          <span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>{t(s.label)}</span></label>;
      })}
      <div className="ai-actions">
        <button type="submit" className="primary-button" disabled={busy || !changed || bad.length > 0}>{busy ? <CircleNotch size={14} className="spin" /> : null}{t('保存')}</button>
      </div>
      {note && <p className={'account-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>}
    </form>}
  </div></div>;
}
