// 为哪些会话写 AI 见解（#1078）：`ai_insight_filter_mode` 和 `ai_insight_filter_list`，从私聊中选择。
// 见解引擎和见解页的「生成见解」只为它们允许的会话写见解；只处理名单里的会话而名单为空时，一个也不写。
import { useEffect, useMemo, useState } from 'react';
import { Check, MagnifyingGlass, WarningCircle } from '@phosphor-icons/react';
import { getExcludeCandidates, type Candidate } from '../api/analytics';
import type { InsightSessionMode, InsightSessions as Sessions } from '../api/settings';
import { Segmented } from '../Segmented';
import { ListSkeleton } from '../Skeleton';
import { reason, SettingRow, SettingsGroup } from './layout';
import { locale, msg, t } from '../i18n';

/** 名单的两种用法，按选项里的顺序。 */
const MODES: { value: InsightSessionMode; label: string }[] = [
  { value: 'whitelist', label: msg('只处理选中的会话') },
  { value: 'blacklist', label: msg('选中的会话除外') },
];

/** 除了保存在名单里的会话，再多列出这么多个；其余的要搜索才找得到。 */
const LISTED = 8;

/** 用一句话说明名单现在的含义，以及它是否一个会话也不允许。 */
function summary({ mode, list }: Sessions): { text: string; none: boolean } {
  const n = list.length.toLocaleString(locale());
  if (mode === 'blacklist') return { text: list.length ? t('为选中的 {n} 个以外的所有私聊写见解。', { n }) : t('为所有私聊写见解。'), none: false };
  return list.length ? { text: t('只为选中的 {n} 个私聊写见解。', { n }), none: false } : { text: t('还没有选会话：灵感引擎和「生成见解」都不会写见解。'), none: true };
}

/**
 * AI 设置里选择会话的一组；`value` 属于表单，与表单的其他内容一起保存。`saved` 名单里的会话排在最前，
 * 勾选时行不会在指针下面移动。
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
    // 已保存、但候选列表里没有的用户名（会话已经不在，或者是用 config set 写入的）也列出来，以便取消
    const known = new Set(people.map(p => p.username));
    const extra: Candidate[] = saved.filter(id => !known.has(id)).map(id => ({ username: id, displayName: id, wechatId: '' }));
    const words = keyword.trim().toLowerCase();
    return [...people, ...extra]
      .filter(p => !words || [p.displayName, p.username, p.wechatId].some(v => v.toLowerCase().includes(words)))
      .sort((a, b) => Number(first.has(b.username)) - Number(first.has(a.username)));
  }, [people, keyword, saved, first]);
  // 已保存的、之后勾选的（不论排在哪里），再加几个
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
