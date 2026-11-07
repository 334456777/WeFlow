// The rest of the settings page (#213): the language of the CLI's output, the AI model API with a test of it, and the
// cache, each part with its size, cleared after showing what goes.
import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { Check, CircleNotch, Key, Plugs, Trash } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import {
  AI_KEYS, CACHE_LABELS, clearCache, getSetting, listCache, planClearCache, setSetting, testAi, unsetSetting,
  type CachePart, type CachePartName,
} from '../api/settings';
import type { Profile } from '../api/setup';
import { formatSize } from '../jobs/TaskCenter';
import { Segmented } from '../Segmented';
import { GridSkeleton } from '../Skeleton';
import { locale, msg, t } from '../i18n';

const reason = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || fallback;
type Note = { text: string; error?: boolean } | null;
const NoteLine = ({ note }: { note: Note }) => note && <p className={'account-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>;

// the system's choice shown through t(); a language by its own name
const LANGS = [{ value: null, label: msg('跟随系统') }, { value: 'zh', label: '中文' }, { value: 'en', label: 'English' }] as const; // i18n-ignore: languages by their own names

/** `lang`: the language of the CLI's output and errors, for the commands the server runs. */
export function LanguageSetting() {
  const [lang, setLang] = useState<string | null | undefined>(undefined);
  const [note, setNote] = useState<Note>(null);
  useEffect(() => {
    const controller = new AbortController();
    getSetting('lang', controller.signal).then(setLang, () => { if (!controller.signal.aborted) setLang(null); });
    return () => controller.abort();
  }, []);
  const choose = async (value: string | null) => {
    const before = lang;
    setLang(value); setNote(null);
    try {
      await (value ? setSetting('lang', value) : unsetSetting('lang'));
      setNote({ text: t('已保存。服务端进程内的读取在重启 weflow serve 后改用新语言。') });
    } catch (problem) {
      setLang(before);
      setNote({ text: reason(problem, t('保存失败，请重试。')), error: true });
    }
  };
  return <div className="setting-row setting-choice"><div><h2>{t('输出语言')}</h2><p>{t('weflow 命令的输出和错误信息用的语言，比如任务失败时显示的原因。')}</p><NoteLine note={note} /></div>
    <Segmented className="theme-group" count={LANGS.length} index={Math.max(0, LANGS.findIndex(l => l.value === (lang ?? null)))} role="group" aria-label={t('输出语言')}>
      {LANGS.map(l => <button type="button" key={l.label} disabled={lang === undefined} aria-pressed={lang !== undefined && (lang ?? null) === l.value} onClick={() => choose(l.value)}>{l.value === null ? t(l.label) : l.label}</button>)}
    </Segmented>
  </div>;
}

type AiForm = Record<(typeof AI_KEYS)[number], string> & { key: string };

/** The AI model API of the insights (`ai_model_api_*`, `ai_insight_enabled`), and `insight test`. */
export function AiSetting({ profile, onSaved }: { profile: Profile | undefined; onSaved(): void }) {
  const [saved, setSaved] = useState<AiForm | null>(null);
  const [form, setForm] = useState<AiForm | null>(null);
  const [busy, setBusy] = useState<'save' | 'test' | null>(null);
  const [note, setNote] = useState<Note>(null);
  useEffect(() => {
    const controller = new AbortController();
    Promise.all(AI_KEYS.map(k => getSetting(k, controller.signal))).then(values => {
      const read = { ...Object.fromEntries(AI_KEYS.map((k, i) => [k, values[i] ?? ''])), key: '' } as AiForm;
      setSaved(read); setForm(read);
    }, problem => { if (!controller.signal.aborted) setNote({ text: reason(problem, t('读不到 AI 设置。')), error: true }); });
    return () => controller.abort();
  }, []);
  const set = (key: keyof AiForm, value: string) => setForm(old => (old ? { ...old, [key]: value } : old));
  const tokensBad = Boolean(form?.ai_model_api_max_tokens.trim()) && !/^\d+$/.test(form!.ai_model_api_max_tokens.trim());
  const changed = Boolean(form && saved && (form.key.trim() || AI_KEYS.some(k => form[k].trim() !== saved[k])));

  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (!form || !saved || tokensBad) return;
    setBusy('save'); setNote(null);
    try {
      for (const k of AI_KEYS) {
        const value = form[k].trim();
        if (value === saved[k]) continue;
        await (value ? setSetting(k, value) : unsetSetting(k));
      }
      // the key goes to the server only, typed in; it is never read back
      if (form.key.trim()) await setSetting('ai_model_api_key', form.key.trim());
      const next = { ...form, key: '' };
      setSaved(next); setForm(next);
      setNote({ text: t('已保存。') });
      onSaved();
    } catch (problem) {
      setNote({ text: reason(problem, t('保存失败，请重试。')), error: true });
    } finally {
      setBusy(null);
    }
  };
  const test = async () => {
    setBusy('test'); setNote(null);
    try { await testAi(); setNote({ text: t('连接成功，模型有回应。') }); }
    catch (problem) { setNote({ text: t('连接失败：{why}', { why: reason(problem, t('没有回应。')) }), error: true }); }
    finally { setBusy(null); }
  };

  return <div className="setting-row setting-accounts"><div><h2>{t('AI 见解')}</h2><p>{t('灵感信箱用的 AI 模型接口。API Key 只保存在服务端，网页只知道它是否已设置。')}</p>
    {!form ? (note ? <NoteLine note={note} /> : <GridSkeleton label={t('正在读取 AI 设置')} cells={4} />) : <form className="ai-form" onSubmit={save}>
      <label className="console-field"><span className="console-field-name">{t('接口地址')}</span>
        <input className="console-input" type="url" spellCheck={false} placeholder="https://api.example.com/v1" value={form.ai_model_api_base_url} onChange={e => set('ai_model_api_base_url', e.target.value)} /></label>
      <label className="console-field"><span className="console-field-name">{t('模型')}</span>
        <input className="console-input" type="text" spellCheck={false} placeholder={t('模型名称')} value={form.ai_model_api_model} onChange={e => set('ai_model_api_model', e.target.value)} /></label>
      <label className="console-field"><span className="console-field-name">API Key</span>
        <span className="token-input"><Key size={16} aria-hidden="true" /><input type="password" autoComplete="off" spellCheck={false} aria-label="API Key"
          placeholder={profile?.aiKey ? t('已设置；留空则不修改') : t('还没有设置')} value={form.key} onChange={e => set('key', e.target.value)} /></span></label>
      <label className="console-field"><span className="console-field-name">{t('单次回复的最多 token')}</span>
        <input className="console-input" type="text" inputMode="numeric" placeholder={t('默认')} value={form.ai_model_api_max_tokens} onChange={e => set('ai_model_api_max_tokens', e.target.value)} />
        {tokensBad && <small className="setting-warning">{t('要填整数。')}</small>}</label>
      <label className="checkbox ai-enabled"><input type="checkbox" aria-label={t('开启 AI 见解')} checked={form.ai_insight_enabled === 'true'} onChange={e => set('ai_insight_enabled', e.target.checked ? 'true' : 'false')} />
        <span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>{t('开启 AI 见解')}</span></label>
      <div className="ai-actions">
        <button type="button" className="secondary-button" disabled={busy !== null || changed} title={changed ? t('先保存，再用保存的设置测试') : undefined} onClick={test}>{busy === 'test' ? <CircleNotch size={12} className="spin" /> : <Plugs size={14} />}{t('测试连接')}</button>
        <button type="submit" className="primary-button" disabled={busy !== null || !changed || tokensBad}>{busy === 'save' ? <CircleNotch size={14} className="spin" /> : null}{t('保存')}</button>
      </div>
      <NoteLine note={note} />
    </form>}
  </div></div>;
}

/** `cache list`, and `cache clear --<part>` after `--dry-run` showed what goes. */
export function CacheSetting() {
  const [parts, setParts] = useState<CachePart[] | null>(null);
  const [round, setRound] = useState(0);
  const [plan, setPlan] = useState<{ part: CachePartName; paths: { path: string; bytes: number }[]; bytes: number } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<Note>(null);
  // the part a note is about: it shows under that part's row, by its button (#346); null under the whole list
  const [noted, setNoted] = useState<CachePartName | null>(null);
  const tell = (text: Note, part: CachePartName | null) => { setNote(text); setNoted(part); };
  useEffect(() => {
    const controller = new AbortController();
    listCache(controller.signal).then(list => setParts(Array.isArray(list) ? list : []), problem => {
      if (!controller.signal.aborted) { setParts([]); tell({ text: reason(problem, t('读不到缓存。')), error: true }, null); }
    });
    return () => controller.abort();
  }, [round]);
  const look = async (part: CachePartName) => {
    setBusy('plan:' + part); tell(null, null);
    try { setPlan({ part, ...(await planClearCache(part)) }); }
    catch (problem) { tell({ text: reason(problem, t('读不到将要清理的内容。')), error: true }, part); }
    finally { setBusy(null); }
  };
  const clear = async () => {
    if (!plan) return;
    setBusy('clear'); tell(null, null);
    try {
      await clearCache(plan.part);
      tell({ text: t('已清理{title}，释放 {size}。', { title: t(CACHE_LABELS[plan.part].title), size: formatSize(plan.bytes) }) }, plan.part);
      setPlan(null); setRound(n => n + 1);
    } catch (problem) {
      tell({ text: reason(problem, t('清理失败，请重试。')), error: true }, plan.part);
    } finally {
      setBusy(null);
    }
  };
  const total = parts?.reduce((sum, p) => sum + p.bytes, 0) ?? 0;

  return <div className="setting-row setting-accounts"><div><h2>{t('缓存')}</h2><p>{parts
      ? t('WeFlow 为读得更快而保存的文件，共 {size}。清理不会动微信自己的文件，需要时会重新生成。', { size: formatSize(total) })
      : t('WeFlow 为读得更快而保存的文件。清理不会动微信自己的文件，需要时会重新生成。')}</p>
    {!parts ? <GridSkeleton label={t('正在读取缓存')} cells={6} /> : <ul className="cache-list">
      {parts.map(p => <li key={p.part}>
        <span className="account-copy"><strong>{CACHE_LABELS[p.part] ? t(CACHE_LABELS[p.part].title) : p.part}</strong><span className="cache-text">{CACHE_LABELS[p.part] && t(CACHE_LABELS[p.part].text)}</span></span>
        <span className="cache-size num">{formatSize(p.bytes)}{p.files ? ' · ' + t('{n} 个文件', { n: p.files.toLocaleString(locale()) }) : ''}</span>
        <button type="button" className="secondary-button" aria-label={t('清理{title}', { title: CACHE_LABELS[p.part] ? t(CACHE_LABELS[p.part].title) : p.part })} disabled={!p.bytes || busy !== null} onClick={() => look(p.part)}>
          {busy === 'plan:' + p.part ? <CircleNotch size={12} className="spin" /> : <Trash size={12} />}{t('清理')}</button>
        {plan && plan.part === p.part && <ClearPlan plan={plan} busy={busy === 'clear'} onCancel={() => setPlan(null)} onClear={clear} />}
        {noted === p.part && <NoteLine note={note} />}
      </li>)}
    </ul>}
    {noted === null && <NoteLine note={note} />}
  </div></div>;
}

/** What `cache clear --dry-run` says would go, under the row of its part, with the way to go on. */
function ClearPlan({ plan, busy, onCancel, onClear }: {
  plan: { part: CachePartName; paths: { path: string; bytes: number }[]; bytes: number }; busy: boolean; onCancel(): void; onClear(): void;
}) {
  return <div className="clear-plan" role="group" aria-label={t('将要清理的内容')}>
    <p>{t('清理{title}：删除下面的文件，共 {bytes}。', { title: t(CACHE_LABELS[plan.part].title), bytes: formatSize(plan.bytes) })}</p>
    <ul>{plan.paths.map(p => <li key={p.path}><span className="num">{formatSize(p.bytes)}</span><span className="account-path" title={p.path}>{p.path}</span></li>)}</ul>
    <div className="clear-plan-actions">
      <button type="button" className="secondary-button" disabled={busy} onClick={onCancel}>{t('取消')}</button>
      <button type="button" className="primary-button danger" disabled={busy} onClick={onClear}>{busy ? <CircleNotch size={14} className="spin" /> : <Trash size={14} />}{t('确认清理')}</button>
    </div>
  </div>;
}
