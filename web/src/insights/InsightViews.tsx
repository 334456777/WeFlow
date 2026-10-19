// The insights page (#210): who has insights beside them, the insights in the middle, and the console's cards to
// see how many, write one now and clear them.
import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { ArrowClockwise, ChatCircleDots, CircleNotch, Copy, Lightbulb, MagnifyingGlass, Sparkle, Trash } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { clearInsights, getInsightLog, reasonLabel, triggerInsight, type InsightContact, type InsightLog, type InsightRecord } from '../api/insights';
import { Avatar } from '../Avatar';
import { formatSessionTime } from '../chat/sessions';
import { HoverTrack } from '../HoverTrack';
import { GridSkeleton, ListSkeleton, MomentsSkeleton } from '../Skeleton';
import type { InsightsState } from './useInsights';

const reason = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || fallback;

/** Beside the insights: all of them, or one conversation's, and words to look for. */
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
  if (state.status === 'idle' || state.status === 'loading') body = <ListSkeleton label="正在读取见解" rows={6} />;
  else if (state.status === 'error') {
    body = <div className="session-message" role="alert"><p>{state.message}</p><button type="button" className="secondary-button" onClick={onRetry}><ArrowClockwise size={14} />重试</button></div>;
  } else if (!contacts.length) body = <p className="session-message">还没有 AI 见解。</p>;
  else {
    body = <div data-roving="vertical" data-roving-items=".conversation-row" className="conversation-list scroll-area" aria-label="有见解的联系人"><HoverTrack items=".conversation-row" />
      {[null, ...contacts].map(c => {
        const id = c?.sessionId ?? null;
        const active = id === selected;
        return <button type="button" key={id ?? '\u0000all'} className={'conversation-row' + (active ? ' active' : '')} aria-pressed={active} onClick={() => onSelect(id)}
          aria-label={(c ? '只看' + c.displayName : '全部见解') + '，' + (c?.count ?? all) + ' 条'}>
          {c ? <Avatar name={c.displayName} url={c.avatarUrl} /> : <span className="avatar avatar-initial" aria-hidden="true"><Lightbulb size={20} /></span>}
          <span className="conversation-copy">
            <span className="conversation-head"><span className="conversation-name">{c?.displayName ?? '全部见解'}</span></span>
            <span className="conversation-preview clamp-1">{(c?.count ?? all).toLocaleString('zh-CN')} 条</span>
          </span>
        </button>;
      })}
    </div>;
  }

  return <div className="session-pane">
    <form className="session-tools" onSubmit={search}>
      <div className="search-input session-search"><MagnifyingGlass size={16} aria-hidden="true" />
        <input type="search" aria-label="搜索见解" placeholder="搜索见解或联系人，回车搜索" value={typed}
          onChange={event => { setTyped(event.target.value); if (!event.target.value && keyword) onKeyword(''); }} />
      </div>
    </form>
    {body}
  </div>;
}

/** How a record came to be, read when asked for. */
function LogDetail({ id }: { id: string }) {
  const [log, setLog] = useState<InsightLog | null | 'error' | undefined>(undefined);
  useEffect(() => {
    const controller = new AbortController();
    getInsightLog(id, controller.signal).then(setLog, () => { if (!controller.signal.aborted) setLog('error'); });
    return () => controller.abort();
  }, [id]);
  if (log === undefined) return <p className="muted"><CircleNotch size={12} className="spin" /> 正在读取…</p>;
  if (log === 'error' || log === null) return <p className="setting-warning">读不到生成过程。</p>;
  return <dl className="insight-log">
    <div><dt>模型</dt><dd>{log.model || '—'}</dd></div>
    <div><dt>用时</dt><dd className="num">{(log.durationMs / 1000).toFixed(1)} 秒</dd></div>
    <div><dt>上下文</dt><dd className="num">{log.contextCount} 条消息</dd></div>
    {log.userPrompt && <div className="wide"><dt>提示</dt><dd><pre>{log.userPrompt}</pre></dd></div>}
    {log.rawOutput && <div className="wide"><dt>模型原文</dt><dd><pre>{log.rawOutput}</pre></dd></div>}
  </dl>;
}

function InsightCard({ record, canChat, onRead, onChat }: { record: InsightRecord; canChat: boolean; onRead(): void; onChat(): void }) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try { await navigator.clipboard.writeText(record.insight); setCopied(true); window.setTimeout(() => setCopied(false), 1500); } catch { /* the browser refused */ }
  };
  return <article className={'insight-card' + (record.read ? '' : ' unread')} aria-label={record.displayName + '的见解'} onClick={() => { if (!record.read) onRead(); }}>
    <header>
      <Avatar name={record.displayName} url={record.avatarUrl} />
      <span className="insight-who"><strong className="clamp-1">{record.displayName}</strong>
        <span><time className="num">{formatSessionTime(Math.floor(record.createdAt / 1000), new Date())}</time> · {reasonLabel(record.triggerReason)}</span></span>
      {!record.read && <span className="insight-dot" aria-label="未读" />}
    </header>
    <p className="insight-text">{record.insight}</p>
    <div className="insight-actions">
      <button type="button" className="text-button" onClick={copy}><Copy size={12} />{copied ? '已复制' : '复制'}</button>
      <button type="button" className="text-button" disabled={!canChat} onClick={onChat}><ChatCircleDots size={12} />查看聊天</button>
      <button type="button" className="text-button" aria-expanded={open} onClick={() => setOpen(o => !o)}>{open ? '收起生成过程' : '生成过程'}</button>
    </div>
    {open && <LogDetail id={record.id} />}
  </article>;
}

export function InsightList({ state, narrowed, canChat, onRead, onChat, onMore }: {
  state: InsightsState;
  narrowed: boolean;
  canChat(session: string): boolean;
  onRead(id: string): void;
  onChat(session: string): void;
  onMore(): void;
}) {
  if (state.status !== 'ready') return state.status === 'error' ? null : <MomentsSkeleton label="正在读取见解" />;
  const { records, total } = state.page;
  if (!records.length) {
    return <div className="pane-message"><p>{narrowed ? '没有符合条件的见解。' : '还没有 AI 见解。'}</p><p className="muted">开启 AI 见解后，weflow serve --insight 会在聊天活跃或久未联系时写下见解；也可以在右侧手动生成一条。</p></div>;
  }
  return <div className="insights" aria-label="见解">
    {records.map(r => <InsightCard key={r.id} record={r} canChat={canChat(r.sessionId)} onRead={() => onRead(r.id)} onChat={() => onChat(r.sessionId)} />)}
    <div className="moments-end">
      {state.more
        ? <button type="button" className="secondary-button" disabled={state.loadingMore} onClick={onMore}>{state.loadingMore ? <><CircleNotch size={14} className="spin" />正在读取</> : '显示更早的见解'}</button>
        : <p>共 {total.toLocaleString('zh-CN')} 条</p>}
      {state.error && <p className="setting-warning" role="alert">{state.error}</p>}
    </div>
  </div>;
}

/** How many, and whether the engine writes any. */
export function InsightsSummary({ state, engine, aiReady }: { state: InsightsState; engine: boolean | undefined; aiReady: boolean | undefined }) {
  if (state.status !== 'ready') return <GridSkeleton label="正在统计见解" cells={3} />;
  const { page } = state;
  return <>
    <dl className="info-grid">
      <div><dt>全部</dt><dd className="num">{page.contacts.reduce((n, c) => n + c.count, 0).toLocaleString('zh-CN')} 条</dd></div>
      <div><dt>今天</dt><dd className="num">{page.todayCount.toLocaleString('zh-CN')} 条</dd></div>
      <div><dt>未读</dt><dd className="num">{page.unreadCount.toLocaleString('zh-CN')} 条</dd></div>
    </dl>
    <p className="card-note">{engine
      ? 'weflow serve 正在运行见解引擎，会自动写下新的见解。'
      : '见解引擎没有运行：用 weflow serve --insight 启动后，才会自动写见解。'}
      {aiReady === false && ' 还没有配置 AI 的 API Key，在设置里填写。'}</p>
  </>;
}

/** `insight trigger`: one insight now, for the conversation chosen or the first one it may write for. */
export function InsightTrigger({ session, name, onDone }: { session: string | null; name: string | null; onDone(): void }) {
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  useEffect(() => setNote(null), [session]);
  const run = async () => {
    setBusy(true); setNote(null);
    try {
      const done = await triggerInsight(session);
      setNote({ text: done.record ? '已为「' + done.record.displayName + '」写下一条见解。' : (done.message || '已完成。') });
      onDone();
    } catch (problem) {
      setNote({ text: reason(problem, '没有生成见解。'), error: true });
    } finally {
      setBusy(false);
    }
  };
  return <>
    <p className="card-note">{session ? '为「' + (name ?? session) + '」' : '为第一个可以写见解的私聊'}立即生成一条见解，要等模型回答，可能需要几十秒。</p>
    <button type="button" className="primary-button large export-pending" disabled={busy} onClick={run}>{busy ? <CircleNotch size={18} className="spin" /> : <Sparkle size={18} />}{busy ? '正在生成' : '生成见解'}</button>
    {note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined}>{note.text}</p>}
  </>;
}

/** `insight clear`, of the conversation chosen or of all, after asking. */
export function InsightClear({ session, name, count, onDone }: { session: string | null; name: string | null; count: number; onDone(): void }) {
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  useEffect(() => { setAsking(false); setNote(null); }, [session]);
  const what = session ? '「' + (name ?? session) + '」的 ' + count + ' 条见解' : '全部 ' + count + ' 条见解';
  const clear = async () => {
    setBusy(true); setNote(null);
    try {
      const done = await clearInsights(session);
      setNote({ text: '已删除 ' + (done.removed ?? count) + ' 条。' });
      setAsking(false);
      onDone();
    } catch (problem) {
      setNote({ text: reason(problem, '删除失败，请重试。'), error: true });
    } finally {
      setBusy(false);
    }
  };
  return <>
    {asking
      ? <div className="clear-plan" role="group" aria-label="确认删除见解"><p>删除{what}？删除后不能恢复。</p>
        <div className="clear-plan-actions"><button type="button" className="secondary-button" disabled={busy} onClick={() => setAsking(false)}>取消</button>
          <button type="button" className="primary-button danger" disabled={busy} onClick={clear}>{busy ? <CircleNotch size={14} className="spin" /> : <Trash size={14} />}确认删除</button></div></div>
      : <button type="button" className="secondary-button" disabled={!count} onClick={() => setAsking(true)}><Trash size={14} />删除{what}</button>}
    {note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined}>{note.text}</p>}
  </>;
}
