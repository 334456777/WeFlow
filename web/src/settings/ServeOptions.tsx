// The options weflow serve starts with (#406), saved in its config file (`serve_*`): each is used when the command
// line does not give it, from the next start of the server. Shown by subject (#776), saved together from the bar under
// them.
import { useEffect, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import { ArrowRight, CircleNotch } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { getSetting, setSetting, unsetSetting } from '../api/settings';
import { GridSkeleton } from '../Skeleton';
import { SaveBar, SettingRow, SettingsGroup, Toggle } from './layout';
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

const KEYS = ['serve_host', ...NUMBERS.map(n => n.key), ...SWITCHES.map(s => s.key), 'autoDownloadWhitelist'];
/** Each key's saved value as text: `''` when none is saved. */
type Form = Record<string, string>;

const wholeNumber = (text: string) => !text.trim() || (/^\d+$/.test(text.trim()) && Number(text) >= 1 && Number(text) <= 65535);
/** A listen address only this machine reaches. */
const loopback = (host: string) => !host || host === 'localhost' || host.startsWith('127.') || host === '::1' || host === '[::1]';

/** `onAi` opens the AI insights, the other half of what makes insights come by themselves. */
export function ServeOptions({ onAi }: { onAi(): void }) {
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
        if (k === 'autoDownloadWhitelist') {
          // the config holds a JSON array; the field shows it comma-separated
          const ids = value.split(',').map(s => s.trim()).filter(Boolean);
          await (ids.length ? setSetting(k, JSON.stringify(ids)) : unsetSetting(k));
        } else {
          await (value ? setSetting(k, value) : unsetSetting(k));
        }
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

  if (!form) {
    return <SettingsGroup>{note ? <p className="account-note setting-warning" role="alert">{note.text}</p> : <GridSkeleton label={t('正在读取启动选项')} cells={4} />}</SettingsGroup>;
  }
  const number = (n: (typeof NUMBERS)[number]) => <label key={n.key} className="console-field"><span className="console-field-name">{t(n.label)}</span>
    <input className="console-input" type="text" inputMode="numeric" placeholder={t(n.fallback)} value={form[n.key]} onChange={e => set(n.key, e.target.value)} />
    {bad.includes(n.key) && <small className="setting-warning">{t('要填 1 到 65535 的整数。')}</small>}</label>;
  const switchRow = (s: (typeof SWITCHES)[number], text?: string, more?: ReactNode) => {
    const on = form[s.key] ? form[s.key] === 'true' : s.fallback;
    // the server's default is saved as nothing
    return <SettingRow key={s.key} name={t(s.label)} text={text}>
      {more}
      <Toggle label={t(s.label)} checked={on} onChange={checked => set(s.key, checked === s.fallback ? '' : String(checked))} />
    </SettingRow>;
  };
  const [port, ...limits] = NUMBERS;
  const [openBrowser, insight, imageDownload, logRequests] = SWITCHES;
  return <form className="serve-form" onSubmit={save}>
    <SettingsGroup title={t('网络')}>
      <div className="settings-fields">
        <label className="console-field"><span className="console-field-name">{t('监听地址')}</span>
          <input className="console-input" type="text" spellCheck={false} placeholder="127.0.0.1" value={form.serve_host} onChange={e => set('serve_host', e.target.value)} /></label>
        {number(port)}
      </div>
      {!loopback(form.serve_host.trim()) && <p className="setting-warning">{t('监听本机以外的地址时，持有令牌的人可以在这台机器上运行任何 weflow 命令。')}</p>}
    </SettingsGroup>
    <SettingsGroup title={t('并发')}>
      <div className="settings-fields">{limits.map(number)}</div>
    </SettingsGroup>
    <SettingsGroup title={t('启动与日志')}>{switchRow(openBrowser)}{switchRow(logRequests)}</SettingsGroup>
    <SettingsGroup title={t('功能')}>
      {switchRow(insight, t('在后台自动生成见解；还要在「AI 见解」里开启并设置模型接口。'),
        <button type="button" className="secondary-button" onClick={onAi}>{t('前往 AI 见解')}<ArrowRight size={12} /></button>)}
      {switchRow(imageDownload)}
      <label className="console-field"><span className="console-field-name">{t('自动下载原图的会话')}</span>
        <input className="console-input" type="text" spellCheck={false} placeholder={t('会话 ID，用逗号分隔')} value={form.autoDownloadWhitelist} onChange={e => set('autoDownloadWhitelist', e.target.value)} />
        <small>{t('留空表示不为任何会话下载原图。')}</small></label>
    </SettingsGroup>
    <SaveBar note={note
      ? <p className={'account-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>
      : changed && <p className="account-note">{t('有未保存的修改，重启 weflow serve 后生效。')}</p>}>
      <button type="submit" className="primary-button" disabled={busy || !changed || bad.length > 0}>{busy ? <CircleNotch size={14} className="spin" /> : null}{t('保存')}</button>
    </SaveBar>
  </form>;
}
