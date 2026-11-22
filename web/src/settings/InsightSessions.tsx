// The conversations the AI insights are written for (#1078): `ai_insight_filter_mode` and `ai_insight_filter_list`,
// chosen among the private chats. The insight engine and the insights page's 生成见解 write only for the
// conversations these allow; with the list alone allowed and empty, they write none.
import { useEffect, useMemo, useState } from 'react';
import { Check, MagnifyingGlass, WarningCircle } from '@phosphor-icons/react';
import { getExcludeCandidates, type Candidate } from '../api/analytics';
import type { InsightSessionMode, InsightSessions as Sessions } from '../api/settings';
import { Segmented } from '../Segmented';
import { ListSkeleton } from '../Skeleton';
import { reason, SettingRow, SettingsGroup } from './layout';
import { locale, msg, t } from '../i18n';

/** The modes, in the order the choice lists them. */
const MODES: { value: InsightSessionMode; label: string }[] = [
  { value: 'whitelist', label: msg('只处理选中的会话') },
  { value: 'blacklist', label: msg('选中的会话除外') },
];

/** Beyond the chosen ones, this many are listed; the rest are found by name. */
const LISTED = 8;

/** What the list means now, in a sentence; whether it allows none. */
function summary({ mode, list }: Sessions): { text: string; none: boolean } {
  const n = list.length.toLocaleString(locale());
  if (mode === 'blacklist') return { text: list.length ? t('为选中的 {n} 个以外的所有私聊写见解。', { n }) : t('为所有私聊写见解。'), none: false };
  return list.length ? { text: t('只为选中的 {n} 个私聊写见解。', { n }), none: false } : { text: t('还没有选会话：灵感引擎和「生成见解」都不会写见解。'), none: true };
}

/**
 * The group of the AI settings that chooses the conversations; `value` is the form's, saved with the rest of it. Those
 * of the `saved` list come first, so that a row does not move under the pointer as it is ticked.
 */
export function InsightSessions({ value, saved, onChange }: { value: Sessions; saved: readonly string[]; onChange(next: Sessions): void }) {
  const [people, setPeople] = useState<Candidate[] | { error: string } | null>(null);
  const [keyword, setKeyword] = useState('');
  const [round, setRound] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setPeople(null);
    getExcludeCandidates(controller.signal).then(list => setPeople(Array.isArray(list) ? list : []), problem => {
      if (!controller.signal.aborted) setPeople({ error: reason(problem, t('读不到私聊列表。')) });
    });
    return () => controller.abort();
  }, [round]);
  const chosen = useMemo(() => new Set(value.list), [value.list]);
  const first = useMemo(() => new Set(saved), [saved]);
  const shown = useMemo(() => {
    if (!people || 'error' in people) return [];
    // a saved id the list does not have (a conversation gone, or typed in with config set) stays, to be removed
    const known = new Set(people.map(p => p.username));
    const extra: Candidate[] = saved.filter(id => !known.has(id)).map(id => ({ username: id, displayName: id, wechatId: '' }));
    const words = keyword.trim().toLowerCase();
    return [...people, ...extra]
      .filter(p => !words || [p.displayName, p.username, p.wechatId].some(v => v.toLowerCase().includes(words)))
      .sort((a, b) => Number(first.has(b.username)) - Number(first.has(a.username)));
  }, [people, keyword, saved, first]);
  // the saved ones, the ones ticked since wherever they are, and a few more
  const listed = shown.filter((p, i) => i < shown.filter(q => first.has(q.username)).length + LISTED || chosen.has(p.username));
  const toggle = (username: string, on: boolean) =>
    onChange({ ...value, list: on ? [...value.list, username] : value.list.filter(id => id !== username) });
  const said = summary(value);

  return <SettingsGroup title={t('会话名单')} note={t('灵感引擎和「生成见解」只为名单允许的私聊写见解，群聊不写。')}>
    <SettingRow name={t('名单的用法')} text={said.none
      ? <span className="setting-warning"><WarningCircle size={14} weight="fill" aria-hidden="true" /> {said.text}</span>
      : said.text} wide>
      <Segmented className="insight-mode-group" count={MODES.length} index={MODES.findIndex(m => m.value === value.mode)} role="group" aria-label={t('名单的用法')}>
        {MODES.map(m => <button type="button" key={m.value} aria-pressed={value.mode === m.value} onClick={() => onChange({ ...value, mode: m.value })}>{t(m.label)}</button>)}
      </Segmented>
    </SettingRow>
    {!people ? <ListSkeleton label={t('正在读取私聊')} rows={4} />
      : 'error' in people ? <div className="insight-sessions">
        <p className="account-note setting-warning" role="alert">{people.error}</p>
        <button type="button" className="secondary-button" onClick={() => setRound(r => r + 1)}>{t('重试')}</button>
      </div>
        : <div className="insight-sessions">
          <label className="token-input"><MagnifyingGlass size={14} aria-hidden="true" />
            <input type="search" aria-label={t('搜索私聊')} placeholder={t('搜索名字或微信号')} value={keyword} onChange={event => setKeyword(event.target.value)} /></label>
          <div className="insight-session-list" role="group" aria-label={t('名单里的会话')}>
            {listed.map(p => <label key={p.username} className="checkbox">
              <input type="checkbox" checked={chosen.has(p.username)} onChange={event => toggle(p.username, event.target.checked)} />
              <span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span className="clamp-1">{p.displayName}</span></label>)}
            {!shown.length && <p className="account-note">{t('没有找到。')}</p>}
          </div>
          {shown.length > listed.length && <p className="account-note">{t('还有 {n} 个，搜索名字或微信号找到他们。', { n: (shown.length - listed.length).toLocaleString(locale()) })}</p>}
        </div>}
  </SettingsGroup>;
}
