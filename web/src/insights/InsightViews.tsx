// 灵感信箱页（#210）：一侧是有见解的联系人，中间是见解，操作控制台的卡片用来查看条数、立即生成一条和删除见解。
import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { ArrowClockwise, ArrowRight, ChatCircleDots, CircleNotch, Copy, Lightbulb, MagnifyingGlass, Sparkle, Trash } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { clearInsights, getInsightLog, reasonLabel, triggerInsight, type InsightContact, type InsightLog, type InsightRecord } from '../api/insights';
import { getInsightSessions, noSessionAllowed } from '../api/settings';
import { Avatar } from '../Avatar';
import { formatSessionTime } from '../sessionKind';
import { HoverTrack } from '../HoverTrack';
import { GridSkeleton, ListSkeleton, MomentsSkeleton } from '../Skeleton';
import type { InsightsState } from './useInsights';
import { SampleNote } from '../shell/sample';
import { locale, t } from '../i18n';

const reason = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || fallback;

/** 见解旁边的列表：选全部见解或一个会话的见解，以及搜索的关键词。 */
export function InsightContacts({ state, selected, onSelect, keyword, onKeyword, onRetry }: {
  state: InsightsState;
  selected: string | null;
  onSelect(session: string | null): void;
  keyword: string;
  onKeyword(keyword: string): void;
  onRetry(): void;
}) {
  const [typed, setTyped] = useState(keyword);
  useEffect(() => setTyped(keyword), [keyword]);
  const search = (event: FormEvent) => { event.preventDefault(); onKeyword(typed.trim()); };
  const contacts: InsightContact[] = state.status === 'ready' ? state.page.contacts : [];
  const all = state.status === 'ready' ? contacts.reduce((n, c) => n + c.count, 0) : 0;

  let body;
  if (state.status === 'idle' || state.status === 'loading') body = <ListSkeleton label={t('正在读取见解')} rows={6} />;
  else if (state.status === 'error') {
    body = <div className="session-message" role="alert"><p>{state.message}</p><button type="button" className="secondary-button" onClick={onRetry}><ArrowClockwise size={14} />{t('重试')}</button></div>;
  } else if (!contacts.length) body = <p className="session-message">{t('还没有 AI 见解。')}</p>;
  else {
    body = <div data-roving="vertical" data-roving-items=".conversation-row" className="conversation-list scroll-area" aria-label={t('有见解的联系人')}><HoverTrack items=".conversation-row" />
      {[null, ...contacts].map(c => {
        const id = c?.sessionId ?? null;
        const active = id === selected;
        return <button type="button" key={id ?? '\u0000all'} className={'conversation-row' + (active ? ' active' : '')} aria-pressed={active} onClick={() => onSelect(id)}
          aria-label={c ? t('只看{name}，{n} 条', { name: c.displayName, n: c.count }) : t('全部见解，{n} 条', { n: all })}>
          {c ? <Avatar name={c.displayName} url={c.avatarUrl} /> : <span className="avatar avatar-initial" aria-hidden="true"><Lightbulb size={20} /></span>}
          <span className="conversation-copy">
            <span className="conversation-head"><span className="conversation-name">{c?.displayName ?? t('全部见解')}</span></span>
            <span className="conversation-preview clamp-1">{t('{n} 条', { n: (c?.count ?? all).toLocaleString(locale()) })}</span>
          </span>
        </button>;
      })}
    </div>;
  }

  return <div className="session-pane">
    <form className="session-tools" onSubmit={search}>
      <div className="search-input session-search"><MagnifyingGlass size={16} aria-hidden="true" />
        <input type="search" aria-label={t('搜索见解')} placeholder={t('搜索见解或联系人，回车搜索')} value={typed}
          onChange={event => { setTyped(event.target.value); if (!event.target.value && keyword) onKeyword(''); }} />
      </div>
      <SampleNote />
    </form>
    {body}
  </div>;
}

/** 一条见解的生成过程，展开时才读取。 */
function LogDetail({ id }: { id: string }) {
  const [log, setLog] = useState<InsightLog | null | 'error' | undefined>(undefined);
  useEffect(() => {
    const controller = new AbortController();
    getInsightLog(id, controller.signal).then(setLog, () => { if (!controller.signal.aborted) setLog('error'); });
    return () => controller.abort();
  }, [id]);
  if (log === undefined) return <p className="muted"><CircleNotch size={12} className="spin" /> {t('正在读取…')}</p>;
  if (log === 'error' || log === null) return <p className="setting-warning">{t('读不到生成过程。')}</p>;
  return <dl className="insight-log">
    <div><dt>{t('模型')}</dt><dd>{log.model || '—'}</dd></div>
    <div><dt>{t('用时')}</dt><dd className="num">{t('{value} 秒', { value: (log.durationMs / 1000).toFixed(1) })}</dd></div>
    <div><dt>{t('上下文')}</dt><dd className="num">{t('{contextCount} 条消息', { contextCount: log.contextCount })}</dd></div>
    {log.userPrompt && <div className="wide"><dt>{t('提示')}</dt><dd><pre>{log.userPrompt}</pre></dd></div>}
    {log.rawOutput && <div className="wide"><dt>{t('模型原文')}</dt><dd><pre>{log.rawOutput}</pre></dd></div>}
  </dl>;
}

function InsightCard({ record, canChat, onRead, onChat }: { record: InsightRecord; canChat: boolean; onRead(): void; onChat(): void }) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try { await navigator.clipboard.writeText(record.insight); setCopied(true); window.setTimeout(() => setCopied(false), 1500); } catch { /* 浏览器拒绝写入剪贴板 */ }
  };
  return <article className={'insight-card' + (record.read ? '' : ' unread')} aria-label={t('{name}的见解', { name: record.displayName })} onClick={() => { if (!record.read) onRead(); }}>
    <header>
      <Avatar name={record.displayName} url={record.avatarUrl} />
      <span className="insight-who"><strong className="clamp-1">{record.displayName}</strong>
        <span><time className="num">{formatSessionTime(Math.floor(record.createdAt / 1000), new Date())}</time> · {reasonLabel(record.triggerReason)}</span></span>
      {!record.read && <span className="insight-dot" aria-label={t('未读')} />}
    </header>
    <p className="insight-text">{record.insight}</p>
    <div className="insight-actions">
      <button type="button" className="text-button" onClick={copy}><Copy size={12} />{copied ? t('已复制') : t('复制')}</button>
      <button type="button" className="text-button" disabled={!canChat} onClick={onChat}><ChatCircleDots size={12} />{t('查看聊天')}</button>
      <button type="button" className="text-button" aria-expanded={open} onClick={() => setOpen(o => !o)}>{open ? t('收起生成过程') : t('生成过程')}</button>
    </div>
    {open && <LogDetail id={record.id} />}
  </article>;
}

/** 读到的见解，最新的在前，按需加载更早的。 */
export function InsightList({ state, narrowed, canChat, onRead, onChat, onMore }: {
  state: InsightsState;
  narrowed: boolean;
  canChat(session: string): boolean;
  onRead(id: string): void;
  onChat(session: string): void;
  onMore(): void;
}) {
  if (state.status !== 'ready') return state.status === 'error' ? null : <MomentsSkeleton label={t('正在读取见解')} />;
  const { records, total } = state.page;
  if (!records.length) {
    return <div className="pane-message"><p>{narrowed ? t('没有符合条件的见解。') : t('还没有 AI 见解。')}</p><p className="muted">{t('开启 AI 见解后，weflow serve --insight 会在聊天活跃或久未联系时写下见解；也可以在右侧手动生成一条。')}</p></div>;
  }
  return <div className="insights" aria-label={t('见解')}>
    {records.map(r => <InsightCard key={r.id} record={r} canChat={canChat(r.sessionId)} onRead={() => onRead(r.id)} onChat={() => onChat(r.sessionId)} />)}
    <div className="moments-end">
      {state.more
        ? <button type="button" className="secondary-button" disabled={state.loadingMore} onClick={onMore}>{state.loadingMore ? <><CircleNotch size={14} className="spin" />{t('正在读取')}</> : t('显示更早的见解')}</button>
        : <p>{t('共 {total} 条', { total: total.toLocaleString(locale()) })}</p>}
      {state.error && <p className="setting-warning" role="alert">{state.error}</p>}
    </div>
  </div>;
}

/** 见解的条数，以及见解引擎是否在自动写见解。 */
export function InsightsSummary({ state, engine, aiReady }: { state: InsightsState; engine: boolean | undefined; aiReady: boolean | undefined }) {
  if (state.status !== 'ready') return <GridSkeleton label={t('正在统计见解')} cells={3} />;
  const { page } = state;
  return <>
    <dl className="info-grid">
      <div><dt>{t('全部')}</dt><dd className="num">{t('{n} 条', { n: page.contacts.reduce((n, c) => n + c.count, 0).toLocaleString(locale()) })}</dd></div>
      <div><dt>{t('今天')}</dt><dd className="num">{t('{n} 条', { n: page.todayCount.toLocaleString(locale()) })}</dd></div>
      <div><dt>{t('未读')}</dt><dd className="num">{t('{n} 条', { n: page.unreadCount.toLocaleString(locale()) })}</dd></div>
    </dl>
    <p className="card-note">{engine
      ? t('weflow serve 正在运行见解引擎，会自动写下新的见解。')
      : t('见解引擎没有运行：用 weflow serve --insight 启动后，才会自动写见解。')}
      {aiReady === false && ' ' + t('还没有配置 AI 的 API Key，在设置里填写。')}</p>
  </>;
}

/**
 * `insight trigger`：立即为选中的会话、或者第一个可以写见解的会话生成一条见解。没有选会话时，
 * 可以写见解的会话由 AI 设置里的会话名单决定（#1078）：只允许名单里的会话而名单是空的时，不请求服务端，
 * 而是说明这一点并给出前往设置的入口。
 */
export function InsightTrigger({ session, name, onDone, onSettings }: {
  session: string | null; name: string | null; onDone(): void;
  /** 打开 AI 设置，会话名单在那里。 */
  onSettings?(): void;
}) {
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const [noneAllowed, setNoneAllowed] = useState(false);
  // 每次尝试都重新读取，因为名单可能已经在设置里改过；读不到名单时交给服务端判断
  const listIsEmpty = () => getInsightSessions().then(noSessionAllowed, () => false).then(none => { setNoneAllowed(none); return none; });
  useEffect(() => {
    setNote(null);
    if (session) setNoneAllowed(false);
    else void listIsEmpty();
  }, [session]);
  const run = async () => {
    if (!session && await listIsEmpty()) return;
    setBusy(true); setNote(null);
    try {
      const done = await triggerInsight(session);
      setNote({ text: done.record ? t('已为「{name}」写下一条见解。', { name: done.record.displayName }) : (done.message || t('已完成。')) });
      onDone();
    } catch (problem) {
      setNote({ text: reason(problem, t('没有生成见解。')), error: true });
    } finally {
      setBusy(false);
    }
  };
  return <>
    <p className="card-note">{session
      ? t('为「{name}」立即生成一条见解，要等模型回答，可能需要几十秒。', { name: name ?? session })
      : t('为第一个可以写见解的私聊立即生成一条见解，要等模型回答，可能需要几十秒。')}</p>
    <button type="button" className="primary-button large export-pending" disabled={busy} onClick={run}>{busy ? <CircleNotch size={18} className="spin" /> : <Sparkle size={18} />}{busy ? t('正在生成') : t('生成见解')}</button>
    {!session && noneAllowed && <div className="insight-none-allowed" role="status">
      <p className="card-note setting-warning">{t('会话名单是空的，不会为任何会话写见解。先在「设置 → AI 见解」的会话名单里选择要写见解的私聊。')}</p>
      {onSettings && <button type="button" className="secondary-button" onClick={onSettings}>{t('前往 AI 见解')}<ArrowRight size={12} /></button>}
    </div>}
    {/* 服务端给的原因可能有多行，`.insight-trigger-note` 保留换行 */}
    {note && <p className={'card-note insight-trigger-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined}>{note.text}</p>}
  </>;
}

/** `insight clear`：确认后删除选中的会话的见解，或者全部见解。 */
export function InsightClear({ session, name, count, onDone }: { session: string | null; name: string | null; count: number; onDone(): void }) {
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  useEffect(() => { setAsking(false); setNote(null); }, [session]);
  const what = session ? t('「{name}」的 {n} 条见解', { name: name ?? session, n: count }) : t('全部 {n} 条见解', { n: count });
  const clear = async () => {
    setBusy(true); setNote(null);
    try {
      const done = await clearInsights(session);
      setNote({ text: t('已删除 {n} 条。', { n: done.removed ?? count }) });
      setAsking(false);
      onDone();
    } catch (problem) {
      setNote({ text: reason(problem, t('删除失败，请重试。')), error: true });
    } finally {
      setBusy(false);
    }
  };
  return <>
    {asking
      ? <div className="clear-plan" role="group" aria-label={t('确认删除见解')}><p>{t('删除{what}？删除后不能恢复。', { what })}</p>
        <div className="clear-plan-actions"><button type="button" className="secondary-button" disabled={busy} onClick={() => setAsking(false)}>{t('取消')}</button>
          <button type="button" className="primary-button danger" disabled={busy} onClick={clear}>{busy ? <CircleNotch size={14} className="spin" /> : <Trash size={14} />}{t('确认删除')}</button></div></div>
      : <button type="button" className="secondary-button" disabled={!count} onClick={() => setAsking(true)}><Trash size={14} />{t('删除{what}', { what })}</button>}
    {note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined}>{note.text}</p>}
  </>;
}
