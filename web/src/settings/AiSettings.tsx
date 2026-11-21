// The AI model API of the insights (#213): its API format (OpenAI's Chat Completions or Anthropic's Messages), the
// address a question goes to, the key, the model (from the service's own list when asked for), and a test that says
// where it asked and what came back; and whether the recent messages go with the question, and how many (#950).
import { useEffect, useId, useState } from 'react';
import type { FormEvent } from 'react';
import { ArrowRight, CheckCircle, CircleNotch, Key, ListBullets, Plugs, WarningCircle } from '@phosphor-icons/react';
import type { ModelApiFormat, ModelInfo, ModelProbe } from '../api/contract.gen';
import { AI_KEYS, getSetting, listModels, setSetting, testAi, unsetSetting } from '../api/settings';
import type { Profile } from '../api/setup';
import { GridSkeleton } from '../Skeleton';
import { NoteLine, reason, SaveBar, SettingRow, SettingsGroup, Toggle, type Note } from './layout';
import { endpointOf, formatInfo, formatOf, FORMATS, SUGGESTED_MODELS } from './modelApi';
import { t } from '../i18n';

type AiForm = Record<(typeof AI_KEYS)[number], string> & { key: string };

/** How the last test went. */
type Test = { running: true } | { probe: ModelProbe } | { error: string };

/** The models the service listed, or why it did not. */
type Models = { running: true } | { list: ModelInfo[] } | { error: string };

/**
 * The AI model API of the insights (`ai_model_api_*`, `ai_insight_enabled`), `insight test` and `insight models`.
 * Insights come by themselves only while weflow serve also runs the insight engine, a startup option: the way there is
 * beside the switch.
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
  const [busy, setBusy] = useState<'save' | null>(null);
  const [note, setNote] = useState<Note>(null);
  const [test, setTest] = useState<Test | null>(null);
  const [models, setModels] = useState<Models | null>(null);
  const suggestions = useId();
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
  // the server takes a count below 1 as its default
  const countBad = Boolean(form?.ai_insight_context_count.trim()) && !/^[1-9]\d*$/.test(form!.ai_insight_context_count.trim());
  const changed = Boolean(form && saved && (form.key.trim() || AI_KEYS.some(k => form[k].trim() !== saved[k])));
  const format = formatOf(form?.ai_model_api_format);
  // choosing the format the saved setting means already leaves the setting as it is (unset reads as openai)
  const choose = (id: ModelApiFormat) => {
    if (id === format) return;
    set('ai_model_api_format', saved && formatOf(saved.ai_model_api_format) === id ? saved.ai_model_api_format : id);
    // the list was of the other format's service
    setModels(null);
  };

  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (!form || !saved || tokensBad || countBad) return;
    setBusy('save'); setNote(null);
    try {
      for (const k of AI_KEYS) {
        const value = form[k].trim();
        if (value === saved[k]) continue;
        await (value ? setSetting(k, value) : unsetSetting(k));
      }
      // the key goes to the server only, typed in; it is never read back
      if (form.key.trim()) await setSetting('ai_model_api_key', form.key.trim());
      setSaved({ ...form, key: '' });
      // what was typed while it saved stays, as a change still to save
      setForm(now => now && { ...now, key: now.key === form.key ? '' : now.key });
      setNote({ text: t('已保存。') });
      setTest(null);
      onSaved();
    } catch (problem) {
      setNote({ text: reason(problem, t('保存失败，请重试。')), error: true });
    } finally {
      setBusy(null);
    }
  };
  const runTest = async () => {
    setTest({ running: true });
    try {
      const reply = await testAi();
      setTest(reply.probe ? { probe: reply.probe } : { error: reply.message || t('没有回应。') });
    } catch (problem) {
      setTest({ error: reason(problem, t('没有回应。')) });
    }
  };
  const fetchModels = async () => {
    setModels({ running: true });
    try { setModels({ list: (await listModels()).models }); }
    catch (problem) { setModels({ error: reason(problem, t('读不到模型列表。')) }); }
  };

  if (!form) return <SettingsGroup>{note ? <NoteLine note={note} /> : <GridSkeleton label={t('正在读取 AI 设置')} cells={4} />}</SettingsGroup>;
  const info = formatInfo(format);
  const chatSent = form.ai_insight_allow_context === 'true';
  const endpoint = endpointOf(format, form.ai_model_api_base_url);
  const testing = test !== null && 'running' in test;
  const listing = models !== null && 'running' in models;
  // what is asked of the server uses the saved settings: a change waits for its save
  const waitForSave = changed ? t('先保存，再用保存的设置') : undefined;
  const listed = models !== null && 'list' in models ? models.list : [];
  const offered = [...new Set([...listed.map(m => m.id), ...SUGGESTED_MODELS[format]])];
  return <form className="ai-form" onSubmit={save}>
    <SettingsGroup title={t('自动生成')} note={t('这里的开关和「启动选项」里的灵感引擎都打开时才自动生成；默认只处理会话名单 ai_insight_filter_list 里的会话，名单为空时不生成。')}>
      <SettingRow name={t('开启 AI 见解')} text={t('关闭时，见解引擎和 insight scan 都不生成见解；测试连接和手动生成不受影响。')}>
        <Toggle label={t('开启 AI 见解')} checked={form.ai_insight_enabled === 'true'} onChange={on => set('ai_insight_enabled', on ? 'true' : 'false')} />
      </SettingRow>
      <SettingRow name={t('灵感引擎')} text={engine === undefined
        ? t('在「启动选项」里打开「运行灵感引擎」，重启 weflow serve 后生效。')
        : engine ? t('weflow serve 正在运行灵感引擎。') : t('weflow serve 没有运行灵感引擎：在「启动选项」里打开，重启后生效。')}>
        <button type="button" className="secondary-button" onClick={onServe}>{t('前往启动选项')}<ArrowRight size={12} /></button>
      </SettingRow>
    </SettingsGroup>

    <SettingsGroup title={t('发送给模型的内容')} note={t('打开的内容会随提问发给上面配置的模型服务。朋友圈和微博还要按会话绑定，需用 weflow config set 设置。')}>
      <SettingRow name={t('最近的聊天内容')} text={chatSent
        ? t('每次生成见解时，发送这个会话最近的聊天记录。')
        : t('没有打开时，模型只收到提问和当前时间，几乎总是回答没有可说的（SKIP）。')}>
        <Toggle label={t('发送最近的聊天内容')} checked={chatSent} onChange={on => set('ai_insight_allow_context', on ? 'true' : 'false')} />
      </SettingRow>
      <div className="settings-fields">
        <label className="console-field"><span className="console-field-name">{t('发送的条数')}</span>
          <input className="console-input" type="text" inputMode="numeric" disabled={!chatSent} placeholder={t('默认 40')} aria-label={t('发送的条数')}
            value={form.ai_insight_context_count} onChange={e => set('ai_insight_context_count', e.target.value)} />
          {countBad ? <small className="setting-warning">{t('要填正整数。')}</small> : <small>{t('条数越多，模型看到的越完整，每次请求也越长。')}</small>}</label>
      </div>
    </SettingsGroup>

    <SettingsGroup title={t('模型接口')} note={t('API Key 只保存在服务端，网页只知道它是否已设置。')}
      actions={<button type="button" className="secondary-button" disabled={testing || busy !== null || changed} title={waitForSave} onClick={runTest}>
        {testing ? <CircleNotch size={12} className="spin" /> : <Plugs size={14} />}{t('测试连接')}</button>}>
      <div className="ai-formats" role="group" aria-label={t('接口格式')}>
        {FORMATS.map(f => <button type="button" key={f.id} className="ai-format as-option" aria-pressed={format === f.id} onClick={() => choose(f.id)}>
          <span className="ai-format-name">{t(f.label)}</span>
          <span className="ai-format-text">{t(f.text)}</span>
          <code className="ai-format-path">{f.path}</code>
        </button>)}
      </div>
      <div className="settings-fields">
        <label className="console-field wide"><span className="console-field-name">{t('接口地址')}</span>
          <input className="console-input" type="url" spellCheck={false} placeholder={info.base} value={form.ai_model_api_base_url} onChange={e => set('ai_model_api_base_url', e.target.value)} />
          <small className="ai-endpoint">{t('请求发到')} <code>POST {endpoint}</code>{!form.ai_model_api_base_url.trim() && <> · {t('留空时用官方地址')}</>}</small></label>
        <label className="console-field"><span className="console-field-name">API Key</span>
          <span className="token-input"><Key size={16} aria-hidden="true" /><input type="password" autoComplete="off" spellCheck={false} aria-label="API Key"
            placeholder={profile?.aiKey ? t('已设置；留空则不修改') : t('还没有设置')} value={form.key} onChange={e => set('key', e.target.value)} /></span></label>
        <label className="console-field"><span className="console-field-name">{t('单次回复的最多 token')}</span>
          <input className="console-input" type="text" inputMode="numeric" placeholder={t('默认 4096')} value={form.ai_model_api_max_tokens} onChange={e => set('ai_model_api_max_tokens', e.target.value)} />
          {tokensBad ? <small className="setting-warning">{t('要填整数。')}</small> : <small>{t('模型先推理时，推理也算在内。')}</small>}</label>
        <div className="console-field wide">
          <label className="console-field-name" htmlFor={suggestions + '-model'}>{t('模型')}</label>
          <div className="ai-model-line">
            <input id={suggestions + '-model'} className="console-input" type="text" spellCheck={false} list={offered.length ? suggestions : undefined}
              placeholder={SUGGESTED_MODELS[format][0] ? t('例如 {model}', { model: SUGGESTED_MODELS[format][0] }) : t('模型名称')}
              value={form.ai_model_api_model} onChange={e => set('ai_model_api_model', e.target.value)} />
            <button type="button" className="secondary-button" disabled={listing || busy !== null || changed} title={waitForSave} onClick={fetchModels}>
              {listing ? <CircleNotch size={12} className="spin" /> : <ListBullets size={14} />}{t('获取模型列表')}</button>
          </div>
          {offered.length > 0 && <datalist id={suggestions}>{offered.map(id => <option key={id} value={id} />)}</datalist>}
          {models !== null && 'error' in models && <small className="setting-warning" role="alert">{models.error}</small>}
          {models !== null && 'list' in models && (models.list.length === 0
            ? <small>{t('服务没有列出模型。')}</small>
            : <ModelList list={models.list} chosen={form.ai_model_api_model.trim()} onChoose={id => set('ai_model_api_model', id)} />)}
        </div>
      </div>
      {test && <TestResult test={test} />}
    </SettingsGroup>
    <SaveBar note={note ? <NoteLine note={note} /> : changed && <p className="account-note">{t('有未保存的修改。')}</p>}>
      <button type="submit" className="primary-button" disabled={busy !== null || !changed || tokensBad || countBad}>{busy === 'save' ? <CircleNotch size={14} className="spin" /> : null}{t('保存')}</button>
    </SaveBar>
  </form>;
}

/** The models of `insight models`, the chosen one pressed; one is chosen by a click. */
function ModelList({ list, chosen, onChoose }: { list: ModelInfo[]; chosen: string; onChoose(id: string): void }) {
  return <div className="ai-models" role="group" aria-label={t('服务提供的模型（{n} 个）', { n: list.length })}>
    {list.map(m => <button type="button" key={m.id} className="ai-model as-row in-well" aria-pressed={m.id === chosen} onClick={() => onChoose(m.id)}>
      <code>{m.id}</code>{m.displayName && m.displayName !== m.id && <span>{m.displayName}</span>}
    </button>)}
  </div>;
}

/** What `insight test` found: the model that answered, how long it took and what it said, or why it failed. */
function TestResult({ test }: { test: Test }) {
  if ('running' in test) return <p className="ai-probe" role="status"><CircleNotch size={14} className="spin" />{t('正在请求模型…')}</p>;
  if ('error' in test) return <p className="ai-probe failed" role="alert"><WarningCircle size={16} weight="fill" aria-hidden="true" />{test.error}</p>;
  const { probe } = test;
  return <div className="ai-probe ok" role="status">
    <p className="ai-probe-head"><CheckCircle size={16} weight="fill" aria-hidden="true" /><strong>{t('已连接')}</strong>
      <span className="ai-probe-facts">{probe.model} · <span className="num">{t('{value} 秒', { value: (probe.durationMs / 1000).toFixed(1) })}</span>
        {probe.retries > 0 && <> · {t('重试 {n} 次后成功', { n: probe.retries })}</>}</span></p>
    <blockquote className="ai-probe-reply">{probe.reply}</blockquote>
    <code className="ai-probe-endpoint">{probe.endpoint}</code>
  </div>;
}
