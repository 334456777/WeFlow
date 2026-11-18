// The AI model API of the insights (#213), with a test of it.
import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { ArrowRight, CircleNotch, Key, Plugs } from '@phosphor-icons/react';
import { AI_KEYS, getSetting, setSetting, testAi, unsetSetting } from '../api/settings';
import type { Profile } from '../api/setup';
import { GridSkeleton } from '../Skeleton';
import { NoteLine, reason, SaveBar, SettingRow, SettingsGroup, Toggle, type Note } from './layout';
import { t } from '../i18n';

type AiForm = Record<(typeof AI_KEYS)[number], string> & { key: string };

/**
 * The AI model API of the insights (`ai_model_api_*`, `ai_insight_enabled`), and `insight test`. Insights come by
 * themselves only while weflow serve also runs the insight engine, a startup option: the way there is beside the switch.
 */
export function AiSetting({ profile, engine, onSaved, onServe }: {
  profile: Profile | undefined;
  /** Whether weflow serve runs the insight engine, when it says. */
  engine: boolean | undefined;
  onSaved(): void;
  /** Open the startup options, where the engine is turned on. */
  onServe(): void;
}) {
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

  if (!form) return <SettingsGroup>{note ? <NoteLine note={note} /> : <GridSkeleton label={t('正在读取 AI 设置')} cells={4} />}</SettingsGroup>;
  return <form className="ai-form" onSubmit={save}>
    <SettingsGroup title={t('自动生成')} note={t('这里的开关和「启动选项」里的灵感引擎都打开时才自动生成；默认只处理会话名单 aiInsightFilterList 里的会话，名单为空时不生成。')}>
      <SettingRow name={t('开启 AI 见解')} text={t('关闭时，见解引擎和 insight scan 都不生成见解；测试连接和手动生成不受影响。')}>
        <Toggle label={t('开启 AI 见解')} checked={form.ai_insight_enabled === 'true'} onChange={on => set('ai_insight_enabled', on ? 'true' : 'false')} />
      </SettingRow>
      <SettingRow name={t('灵感引擎')} text={engine === undefined
        ? t('在「启动选项」里打开「运行灵感引擎」，重启 weflow serve 后生效。')
        : engine ? t('weflow serve 正在运行灵感引擎。') : t('weflow serve 没有运行灵感引擎：在「启动选项」里打开，重启后生效。')}>
        <button type="button" className="secondary-button" onClick={onServe}>{t('前往启动选项')}<ArrowRight size={12} /></button>
      </SettingRow>
    </SettingsGroup>
    <SettingsGroup title={t('模型接口')} note={t('API Key 只保存在服务端，网页只知道它是否已设置。')}
      actions={<button type="button" className="secondary-button" disabled={busy !== null || changed} title={changed ? t('先保存，再用保存的设置测试') : undefined} onClick={test}>{busy === 'test' ? <CircleNotch size={12} className="spin" /> : <Plugs size={14} />}{t('测试连接')}</button>}>
      <div className="settings-fields">
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
      </div>
    </SettingsGroup>
    <SaveBar note={note ? <NoteLine note={note} /> : changed && <p className="account-note">{t('有未保存的修改。')}</p>}>
      <button type="submit" className="primary-button" disabled={busy !== null || !changed || tokensBad}>{busy === 'save' ? <CircleNotch size={14} className="spin" /> : null}{t('保存')}</button>
    </SaveBar>
  </form>;
}
