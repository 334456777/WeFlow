// My footprint (#209): over a range of days, how many people I talked with and replied to and how often I was
// @-mentioned, the chats and mentions in the order they happened (each opening the message), and the groups that
// mentioned me; the console cards write the AI's summary and export the statistics.
import { Fragment, useMemo, useState } from 'react';
import { ArrowSquareOut, At, CircleNotch, DownloadSimple, MagnifyingGlass, Sparkle } from '@phosphor-icons/react';
import { Avatar } from '../Avatar';
import { DateRange } from '../calendar/DateRange';
import { contactList, type Contact } from '../api/contacts';
import { currentScope } from '../api/scope';
import { FOOTPRINT_FORMATS, footprintExport, getFootprint, presetRanges, summarizeFootprint, timeline, type Footprint, type FootprintFormat, type FootprintRange } from '../api/footprint';
import { setSetting } from '../api/settings';
import { n } from '../analytics/AnalyticsViews';
import { Failed } from '../reports/AnnualReport';
import { runJob } from '../jobs/useJobs';
import { Segmented } from '../Segmented';
import { GridSkeleton, ListSkeleton } from '../Skeleton';
import { useRead } from '../useRead';
import { t } from '../i18n';

/** A group message's text without the `sender:` line it starts with. */
export const mentionText = (content: string) => content.replace(/^[^\n:]{1,64}:\n/, '').trim() || t('[空消息]');

const clock = (seconds: number) => new Date(seconds * 1000).toLocaleTimeString('zh-CN', { hour12: false, hour: '2-digit', minute: '2-digit' });
const lasted = (seconds: number) => (seconds < 60 ? t('不到 1 分钟') : seconds < 3600 ? t('{n} 分钟', { n: Math.round(seconds / 60) }) : t('{n} 小时', { n: Math.round(seconds / 360) / 10 }));

/** Names and avatars of people and groups, from the contact list. */
export function useNames(): (username: string) => Contact | undefined {
  const [contacts] = useRead('contacts:' + currentScope(), () => contactList());
  return useMemo(() => {
    const byName = new Map((contacts.status === 'ready' ? contacts.value : []).map(c => [c.username, c]));
    return (username: string) => byName.get(username);
  }, [contacts]);
}

/** The ranges to pick from, and the days of a range of one's own. */
export function FootprintRanges({ range, onRange }: { range: FootprintRange; onRange(range: FootprintRange): void }) {
  const presets = presetRanges();
  const custom = !presets.some(p => p.label === range.label);
  return <div className="footprint-ranges">
    <div className="choice-chips" role="group" aria-label={t('时间范围')}>
      {presets.map(p => <button type="button" key={p.label} className="chip" aria-pressed={range.label === p.label} onClick={() => onRange(p)}>{p.label}</button>)}
      <button type="button" className="chip" aria-pressed={custom} onClick={() => { if (!custom) onRange({ ...range, label: range.from === range.to ? range.from : t('{from} 至 {to}', { from: range.from, to: range.to }) }); }}>{t('自定义')}</button>
    </div>
    {custom && <DateRange label={t('足迹的时间范围')} from={range.from} to={range.to}
      onChange={r => r.from && onRange({ from: r.from, to: r.to || r.from, label: r.to && r.to !== r.from ? t('{from} 至 {to}', { from: r.from, to: r.to }) : r.from })} />}
  </div>;
}

export function FootprintView({ data, keyword, onKeyword, names, onOpen }: {
  data: Footprint; keyword: string; onKeyword(keyword: string): void;
  names(username: string): Contact | undefined; onOpen(session: string, localId: number, createTime: number): void;
}) {
  const s = data.summary;
  const name = (username: string) => names(username)?.displayName ?? username;
  const words = keyword.trim().toLowerCase();
  const events = timeline(data).filter(e => !words || (e.kind === 'chat'
    ? name(e.segment.session_id).toLowerCase().includes(words)
    : name(e.mention.session_id).toLowerCase().includes(words) || name(e.mention.sender_username).toLowerCase().includes(words) || e.mention.message_content.toLowerCase().includes(words)));
  let lastDay = '';
  return <div className="analytics-page footprint">
    <section className="analytics-block" aria-label={t('总览')}>
      <dl className="info-grid analytics-numbers">
        <div><dt>{t('给我发消息的人')}</dt><dd className="num">{t('{private_inbound_people} 人', { private_inbound_people: n(s.private_inbound_people) })}</dd></div>
        <div><dt>{t('我回复了')}</dt><dd className="num">{t('{private_replied_people} 人 · {round}%', { private_replied_people: n(s.private_replied_people), round: Math.round(s.private_reply_rate * 100) })}</dd></div>
        <div><dt>{t('我主动联系')}</dt><dd className="num">{t('{private_outbound_people} 人', { private_outbound_people: n(s.private_outbound_people) })}</dd></div>
        <div><dt>{t('@我')}</dt><dd className="num">{t('{mention_count} 次', { mention_count: n(s.mention_count) })}</dd></div>
        <div><dt>{t('涉及群聊')}</dt><dd className="num">{t('{mention_group_count} 个', { mention_group_count: n(s.mention_group_count) })}</dd></div>
      </dl>
      {data.diagnostics.truncated && <p className="card-note">{t('记录太多，只统计了一部分。可以把时间范围缩短一些。')}</p>}
    </section>

    {data.mention_groups.length > 0 && <section className="analytics-block" aria-label={t('@我的群')}>
      <h2>{t('@我的群')}</h2>
      <ul className="word-chips">{[...data.mention_groups].sort((a, b) => b.count - a.count).map(g =>
        <li key={g.session_id}><span className="clamp-1">{name(g.session_id)}</span><small className="num">{n(g.count)}</small></li>)}</ul>
    </section>}

    <section className="analytics-block" aria-label={t('联络时间线')}>
      <div className="analytics-heading"><h2>{t('联络时间线')}</h2>
        <label className="token-input footprint-search"><MagnifyingGlass size={14} aria-hidden="true" />
          <input type="search" aria-label={t('筛选时间线')} placeholder={t('联系人、群聊或内容')} value={keyword} onChange={event => onKeyword(event.target.value)} /></label></div>
      {events.length
        ? <ol className="footprint-timeline">{events.map(e => {
          const day = new Date(e.at * 1000).toLocaleDateString('zh-CN', { month: 'long', day: 'numeric', weekday: 'short' });
          const head = day !== lastDay ? <li className="footprint-day" aria-hidden="true">{day}</li> : null;
          lastDay = day;
          if (e.kind === 'chat') {
            const g = e.segment;
            const who = names(g.session_id);
            return <Fragment key={'c' + g.session_id + g.segment_index}>{head}<li className="footprint-event">
              <Avatar name={who?.displayName ?? g.session_id} url={who?.avatarUrl} />
              <span className="rank-copy">
                <span className="rank-line"><strong>{name(g.session_id)}</strong><span className="num cache-text">{clock(g.start_ts)}{g.end_ts > g.start_ts ? '–' + clock(g.end_ts) : ''}</span></span>
                <span className="cache-text">{t('收到 {incoming} · 发出 {outgoing}', { incoming: n(g.incoming_count), outgoing: n(g.outgoing_count) })} · {lasted(g.end_ts - g.start_ts)}{g.incoming_count > 0 && ' · ' + (g.replied ? t('已回复') : t('没有回复'))}</span>
              </span>
              <button type="button" className="icon-button" aria-label={t('打开和{name}的这段聊天', { name: name(g.session_id) })} title={t('打开聊天')} onClick={() => onOpen(g.session_id, g.anchor_local_id, g.anchor_create_time)}><ArrowSquareOut size={14} /></button>
            </li></Fragment>;
          }
          const m = e.mention;
          const sender = names(m.sender_username)?.displayName;
          return <Fragment key={'m' + m.session_id + m.local_id}>{head}<li className="footprint-event mention">
            <span className="avatar avatar-initial" aria-hidden="true"><At size={18} /></span>
            <span className="rank-copy">
              <span className="rank-line"><strong>{name(m.session_id)}</strong><span className="num cache-text">{clock(m.create_time)}</span></span>
              <span className="cache-text clamp-2">{sender ? sender + '：' : ''}{mentionText(m.message_content)}</span>
            </span>
            <button type="button" className="icon-button" aria-label={t('打开{name}里的这条消息', { name: name(m.session_id) })} title={t('打开聊天')} onClick={() => onOpen(m.session_id, m.local_id, m.create_time)}><ArrowSquareOut size={14} /></button>
          </li></Fragment>;
        })}</ol>
        : <p className="muted">{words ? t('没有符合的联络。') : t('这段时间没有私聊，也没有人 @我。')}</p>}
    </section>
  </div>;
}

export function FootprintPage({ range, onRange, keyword, onKeyword, onOpen, onData }: {
  range: FootprintRange; onRange(range: FootprintRange): void; keyword: string; onKeyword(keyword: string): void;
  onOpen(session: string, localId: number, createTime: number): void; onData(data: Footprint | null): void;
}) {
  const names = useNames();
  const [state, retry] = useRead('footprint:' + range.from + ':' + range.to, async signal => {
    onData(null);
    const data = await getFootprint(range, signal);
    onData(data);
    return data;
  });
  return <>
    <FootprintRanges range={range} onRange={onRange} />
    {state.status === 'error' ? <Failed title={t('读取足迹失败')} message={state.message} onRetry={retry} />
      : state.status === 'loading' ? <div className="analytics-page" aria-busy="true"><GridSkeleton label={t('正在统计{range}的足迹', { range: range.label })} cells={5} /><ListSkeleton label={t('正在读取联络时间线')} rows={5} /></div>
        : <FootprintView data={state.value} keyword={keyword} onKeyword={onKeyword} names={names} onOpen={onOpen} />}
  </>;
}

/** The console card: the AI's few sentences on the range; it is turned on here, the model is set in the settings. */
export function FootprintAi({ range, data, aiReady }: { range: FootprintRange; data: Footprint | null; aiReady?: boolean }) {
  const names = useNames();
  const [text, setText] = useState<{ range: string; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean; enable?: boolean } | null>(null);
  const write = async () => {
    if (!data) return;
    setBusy(true); setNote(null);
    try {
      setText({ range: range.label, text: await summarizeFootprint(range, data, s => names(s)?.displayName ?? s) });
    } catch (problem) {
      const message = (problem instanceof Error && problem.message) || t('没有生成，请重试。');
      // the server's answer, in its language, when the summary is not turned on
      setNote({ text: message, error: true, enable: /AI 足迹总结|AI Footprint summary/i.test(message) });
    } finally {
      setBusy(false);
    }
  };
  const enable = async () => {
    try { await setSetting('aiFootprintEnabled', 'true'); setNote({ text: t('已开启，可以生成了。') }); } catch (problem) { setNote({ text: (problem instanceof Error && problem.message) || t('没有开启，请重试。'), error: true }); }
  };
  return <div className="excluded-people">
    {aiReady === false && <p className="card-note">{t('还没有填写 AI 的 API Key，先到「设置 › 常规 › AI 见解」里填写。')}</p>}
    <button type="button" className="secondary-button" disabled={!data || busy} onClick={write}>{busy ? <CircleNotch size={12} className="spin" /> : <Sparkle size={12} />}{t('总结「{range}」', { range: range.label })}</button>
    {text && <blockquote className="footprint-ai" aria-label={t('AI 足迹总结')}><span className="report-label">{text.range}</span>{text.text}</blockquote>}
    {note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>}
    {note?.enable && <button type="button" className="secondary-button" onClick={enable}>{t('开启 AI 足迹总结')}</button>}
    <p className="card-note">{t('会把这段时间的人数，以及最多 12 段私聊和 12 个群的名字和条数发给你设置的 AI 模型，不发送消息内容。')}</p>
  </div>;
}

/** The console card: the range as a file kept by the server, its statistics as JSON or its timeline as CSV or Excel. */
export function FootprintExport({ range }: { range: FootprintRange }) {
  const [format, setFormat] = useState<FootprintFormat>('json');
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const start = async () => {
    setBusy(true); setNote(null);
    try {
      await runJob(t('导出足迹 · {range}', { range: range.label }), 'POST', 'export/footprint', footprintExport(range, format));
      setNote({ text: t('已加入任务中心，完成后在那里下载。') });
    } catch (problem) {
      setNote({ text: (problem instanceof Error && problem.message) || t('导出没有开始，请重试。'), error: true });
    } finally {
      setBusy(false);
    }
  };
  return <div className="excluded-people">
    <Segmented className="format-group" count={FOOTPRINT_FORMATS.length} index={FOOTPRINT_FORMATS.findIndex(f => f.value === format)} role="group" aria-label={t('导出格式')}>
      {FOOTPRINT_FORMATS.map(f => <button type="button" key={f.value} aria-pressed={format === f.value} onClick={() => setFormat(f.value)}>{f.label}</button>)}
    </Segmented>
    <button type="button" className="secondary-button" disabled={busy} onClick={start}>{busy ? <CircleNotch size={12} className="spin" /> : <DownloadSimple size={12} />}{t('导出「{range}」为 {format}', { range: range.label, format: FOOTPRINT_FORMATS.find(f => f.value === format)?.label ?? '' })}</button>
    <p className="card-note">{format === 'json' ? t('包含人数、每段私聊和每条 @我 的消息。') : t('时间线：每段私聊、每条 @我 的消息各占一行。')}</p>
    {note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>}
  </div>;
}
