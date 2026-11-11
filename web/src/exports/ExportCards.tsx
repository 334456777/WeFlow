// The export page's cards (#211): how the conversations picked are exported, one job each with `export messages`,
// and the list of conversations itself with `export sessions`. The jobs go to the task centre.
import { useState } from 'react';
import { Check, DownloadSimple } from '@phosphor-icons/react';
import type { ChatSession } from '../api/chat';
import { DateRange } from '../calendar/DateRange';
import { runJob } from '../jobs/useJobs';
import { Segmented } from '../Segmented';
import { nameOf } from './ExportPicker';
import { msg, t } from '../i18n';

/** The formats of `export messages --format`, as the page names them. */
export const MESSAGE_FORMATS = [
  { value: 'html', label: 'HTML' }, { value: 'txt', label: 'TXT' }, { value: 'excel', label: 'Excel' },
  { value: 'json', label: 'JSON' }, { value: 'arkme-json', label: 'Arkme JSON' }, { value: 'chatlab', label: 'ChatLab' },
  { value: 'chatlab-jsonl', label: 'ChatLab JSONL' }, { value: 'weclone', label: 'WeClone' }, { value: 'sql', label: 'SQL' },
] as const;
export type MessageFormat = typeof MESSAGE_FORMATS[number]['value'];

/** Formats that write the same extension, of which an export takes one. */
const CLASHES: readonly MessageFormat[][] = [['json', 'arkme-json']];

/**
 * The formats picked after `value` was pressed (#342): taken off, unless it is the last one; or put on, in the
 * list's order, instead of one writing the same extension.
 */
export function toggleFormat(formats: readonly MessageFormat[], value: MessageFormat): MessageFormat[] {
  // the last one too: the export then waits for a format (#353)
  if (formats.includes(value)) return formats.filter(f => f !== value);
  const clash = CLASHES.find(c => c.includes(value)) ?? [];
  const next = new Set([...formats.filter(f => !clash.includes(f)), value]);
  return MESSAGE_FORMATS.map(f => f.value).filter(f => next.has(f));
}

/** Several files come out, or media with them: the server packs them into one zip (`--zip`). */
export const packed = (formats: readonly MessageFormat[], media: boolean) => formats.length > 1 || media;

export const formatsLabel = (formats: readonly MessageFormat[]) => formats.map(formatLabel).join('、');

/** The formats as chips, several at once (#342). */
export function FormatChips({ formats, onChange }: { formats: readonly MessageFormat[]; onChange(formats: MessageFormat[]): void }) {
  return <div className="choice-chips" role="group" aria-label={t('导出格式（可多选）')}>
    {MESSAGE_FORMATS.map(f => <button type="button" key={f.value} className="chip" aria-pressed={formats.includes(f.value)} onClick={() => onChange(toggleFormat(formats, f.value))}>{f.label}</button>)}
  </div>;
}

/** The media `export messages --media` copies, as the page names them. Labels are shown through t(). */
export const MEDIA_KINDS = [
  { value: 'image', label: msg('图片') }, { value: 'voice', label: msg('语音') }, { value: 'video', label: msg('视频') },
  { value: 'emoji', label: msg('表情') }, { value: 'file', label: msg('文件') },
] as const;
export type MediaKind = typeof MEDIA_KINDS[number]['value'];

/** The media picked after `value` was pressed: taken off, or put on in the list's order. */
export function toggleMedia(kinds: readonly MediaKind[], value: MediaKind): MediaKind[] {
  const next = new Set(kinds.includes(value) ? kinds.filter(k => k !== value) : [...kinds, value]);
  return MEDIA_KINDS.map(k => k.value).filter(k => next.has(k));
}

export const mediaLabel = (kinds: readonly MediaKind[]) => kinds.map(k => t(MEDIA_KINDS.find(m => m.value === k)!.label)).join('、');

/** What the media chips do. Shown through t(). */
export const MEDIA_NOTE = msg('选中的媒体放进 media 文件夹，和导出文件一起打包成 zip');

/** The media as chips, like the formats: any of them, or none. */
export function MediaChips({ kinds, onChange, label }: { kinds: readonly MediaKind[]; onChange(kinds: MediaKind[]): void; label: string }) {
  return <div className="choice-chips" role="group" aria-label={label}>
    {MEDIA_KINDS.map(k => <button type="button" key={k.value} className="chip" aria-pressed={kinds.includes(k.value)} onClick={() => onChange(toggleMedia(kinds, k.value))}>{t(k.label)}</button>)}
  </div>;
}

const NAMES = [{ value: 'group-nickname', label: msg('群昵称') }, { value: 'remark', label: msg('备注') }, { value: 'nickname', label: msg('昵称') }] as const;
type NameChoice = typeof NAMES[number]['value'];

export interface BatchOptions {
  formats: MessageFormat[];
  from: string;
  to: string;
  names: NameChoice;
  media: MediaKind[];
  compact: boolean;
}

export const DEFAULT_BATCH: BatchOptions = { formats: ['html'], from: '', to: '', names: 'group-nickname', media: [], compact: false };

/** The parameters of `export messages` for one conversation. */
export function exportParams(session: string, options: BatchOptions) {
  return {
    'session-id': session, format: options.formats, start: options.from || undefined, end: options.to || undefined,
    'display-name': options.names === 'group-nickname' ? undefined : options.names,
    media: options.media.length ? options.media : undefined,
    'excel-compact': options.formats.includes('excel') && options.compact ? true : undefined,
    zip: packed(options.formats, options.media.length > 0) ? true : undefined,
  };
}

export const formatLabel = (format: MessageFormat) => MESSAGE_FORMATS.find(f => f.value === format)!.label;

export function batchSummary(picked: number, options: BatchOptions): string {
  return (picked ? t('{n} 个会话', { n: picked }) : t('未选会话')) + ' · ' + formatsLabel(options.formats) + (options.media.length ? ' · ' + t('含{kinds}', { kinds: mediaLabel(options.media) }) : '');
}

export function BatchExport({ picked, options, onChange }: {
  picked: ChatSession[];
  options: BatchOptions;
  onChange(options: BatchOptions): void;
}) {
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const set = (next: Partial<BatchOptions>) => { onChange({ ...options, ...next }); setNote(null); };
  const start = async () => {
    setBusy(true); setNote(null);
    let queued = 0;
    try {
      // one job each, in the order of the list; the server runs them one after another
      for (const session of picked) {
        await runJob(t('导出 {name} · {formats}', { name: nameOf(session), formats: formatsLabel(options.formats) }), 'POST', 'export/messages', exportParams(session.username, options));
        queued += 1;
      }
      setNote({ text: t('已加入 {n} 个导出任务，在任务中心查看进度和下载文件。', { n: queued }) });
    } catch (problem) {
      const why = problem instanceof Error && problem.message ? problem.message : t('请重试');
      setNote({ text: queued ? t('已加入 {n} 个，其余没有开始：{why}', { n: queued, why }) : t('导出没有开始：{why}', { why }), error: true });
    } finally {
      setBusy(false);
    }
  };
  return <>
    <div className="form-row format-row"><span className="field-label">{t('导出格式')}</span>
      <FormatChips formats={options.formats} onChange={formats => set({ formats })} />
    </div>
    <div className="form-row"><span className="field-label">{t('时间范围')}</span><DateRange label={t('批量导出的时间范围')} from={options.from} to={options.to} onChange={range => set(range)} /></div>
    <div className="form-row"><span className="field-label">{t('发送者名称')}</span>
      <Segmented className="format-group" count={NAMES.length} index={NAMES.findIndex(n => n.value === options.names)} role="group" aria-label={t('发送者名称')}>
        {NAMES.map(n => <button type="button" key={n.value} aria-pressed={options.names === n.value} onClick={() => set({ names: n.value })}>{t(n.label)}</button>)}
      </Segmented>
    </div>
    <div className="form-row format-row"><span className="field-label">{t('附带媒体')}</span>
      <MediaChips kinds={options.media} label={t('附带的媒体（可多选）')} onChange={media => set({ media })} />
      <small className="field-note">{t(MEDIA_NOTE)}</small>
    </div>
    {options.formats.includes('excel') && <label className="checkbox"><input type="checkbox" aria-label={t('Excel 精简列')} checked={options.compact} onChange={event => set({ compact: event.target.checked })} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>{t('精简列')} <small>{t('时间、发送者、类型、内容')}</small></span></label>}
    <button type="button" className="primary-button large export-pending" disabled={!options.formats.length || !picked.length || busy} onClick={start}><DownloadSimple size={18} />{!options.formats.length ? t('选择导出格式') : picked.length ? t('导出 {n} 个会话', { n: picked.length }) : t('先在左侧选择会话')}</button>
    {note && <p className="card-note" role={note.error ? 'alert' : 'status'}>{note.text}</p>}
    <p className="card-note">{t('每个会话一个任务，在服务端依次运行；完成后在任务中心下载。')}</p>
  </>;
}

/** `export sessions`: the list of the conversations picked, or of all of them. */
export function SessionsExport({ picked }: { picked: ChatSession[] }) {
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const start = () => {
    setNote(null);
    runJob(picked.length ? t('导出会话列表 · {n} 个', { n: picked.length }) : t('导出会话列表 · 全部'), 'POST', 'export/sessions', { session: picked.length ? picked.map(s => s.username) : undefined })
      .then(() => setNote({ text: t('已加入任务中心。') }), problem => setNote({ text: problem instanceof Error && problem.message ? problem.message : t('导出没有开始，请重试。'), error: true }));
  };
  return <>
    <p className="card-lead">{picked.length ? t('把已选的 {n} 个会话的列表（会话表的每一行）导出为 JSON。', { n: picked.length }) : t('把全部会话的列表（会话表的每一行）导出为 JSON。')}</p>
    <button type="button" className="secondary-button" onClick={start}><DownloadSimple size={14} />{t('导出会话列表')}</button>
    {note && <p className="card-note" role={note.error ? 'alert' : 'status'}>{note.text}</p>}
  </>;
}
