// The export page's cards (#211): how the conversations picked are exported, one job each with `export messages`,
// and the list of conversations itself with `export sessions`. The jobs go to the task centre.
import { useState } from 'react';
import { Check, DownloadSimple } from '@phosphor-icons/react';
import type { ChatSession } from '../api/chat';
import { DateRange } from '../calendar/DateRange';
import { runJob } from '../jobs/useJobs';
import { Segmented } from '../Segmented';
import { nameOf } from './ExportPicker';

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
  return <div className="choice-chips" role="group" aria-label="导出格式（可多选）">
    {MESSAGE_FORMATS.map(f => <button type="button" key={f.value} className="chip" aria-pressed={formats.includes(f.value)} onClick={() => onChange(toggleFormat(formats, f.value))}>{f.label}</button>)}
  </div>;
}

/** What the media box says it does. */
export const MEDIA_NOTE = '图片、语音、视频与表情放进 media 文件夹，和导出文件一起打包成 zip';

const NAMES = [{ value: 'group-nickname', label: '群昵称' }, { value: 'remark', label: '备注' }, { value: 'nickname', label: '昵称' }] as const;
type NameChoice = typeof NAMES[number]['value'];

export interface BatchOptions {
  formats: MessageFormat[];
  from: string;
  to: string;
  names: NameChoice;
  media: boolean;
  compact: boolean;
}

export const DEFAULT_BATCH: BatchOptions = { formats: ['html'], from: '', to: '', names: 'group-nickname', media: false, compact: false };

/** The parameters of `export messages` for one conversation. */
export function exportParams(session: string, options: BatchOptions) {
  return {
    'session-id': session, format: options.formats, start: options.from || undefined, end: options.to || undefined,
    'display-name': options.names === 'group-nickname' ? undefined : options.names,
    media: options.media ? 'all' : undefined,
    'excel-compact': options.formats.includes('excel') && options.compact ? true : undefined,
    zip: packed(options.formats, options.media) ? true : undefined,
  };
}

export const formatLabel = (format: MessageFormat) => MESSAGE_FORMATS.find(f => f.value === format)!.label;

export function batchSummary(picked: number, options: BatchOptions): string {
  return (picked ? picked + ' 个会话' : '未选会话') + ' · ' + formatsLabel(options.formats) + (options.media ? ' · 含媒体' : '');
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
        await runJob('导出 ' + nameOf(session) + ' · ' + formatsLabel(options.formats), 'POST', 'export/messages', exportParams(session.username, options));
        queued += 1;
      }
      setNote({ text: '已加入 ' + queued + ' 个导出任务，在任务中心查看进度和下载文件。' });
    } catch (problem) {
      const why = problem instanceof Error && problem.message ? problem.message : '请重试';
      setNote({ text: (queued ? '已加入 ' + queued + ' 个，其余没有开始：' : '导出没有开始：') + why, error: true });
    } finally {
      setBusy(false);
    }
  };
  return <>
    <div className="form-row format-row"><span className="field-label">导出格式</span>
      <FormatChips formats={options.formats} onChange={formats => set({ formats })} />
    </div>
    <div className="form-row"><span className="field-label">时间范围</span><DateRange label="批量导出的时间范围" from={options.from} to={options.to} onChange={range => set(range)} /></div>
    <div className="form-row"><span className="field-label">发送者名称</span>
      <Segmented className="format-group" count={NAMES.length} index={NAMES.findIndex(n => n.value === options.names)} role="group" aria-label="发送者名称">
        {NAMES.map(n => <button type="button" key={n.value} aria-pressed={options.names === n.value} onClick={() => set({ names: n.value })}>{n.label}</button>)}
      </Segmented>
    </div>
    <label className="checkbox"><input type="checkbox" aria-label="附带媒体文件" checked={options.media} onChange={event => set({ media: event.target.checked })} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>附带媒体文件 <small>{MEDIA_NOTE}</small></span></label>
    {options.formats.includes('excel') && <label className="checkbox"><input type="checkbox" aria-label="Excel 精简列" checked={options.compact} onChange={event => set({ compact: event.target.checked })} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>精简列 <small>时间、发送者、类型、内容</small></span></label>}
    <button type="button" className="primary-button large export-pending" disabled={!options.formats.length || !picked.length || busy} onClick={start}><DownloadSimple size={18} />{!options.formats.length ? '选择导出格式' : picked.length ? '导出 ' + picked.length + ' 个会话' : '先在左侧选择会话'}</button>
    {note && <p className="card-note" role={note.error ? 'alert' : 'status'}>{note.text}</p>}
    <p className="card-note">每个会话一个任务，在服务端依次运行；完成后在任务中心下载。</p>
  </>;
}

/** `export sessions`: the list of the conversations picked, or of all of them. */
export function SessionsExport({ picked }: { picked: ChatSession[] }) {
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const start = () => {
    setNote(null);
    runJob(picked.length ? '导出会话列表 · ' + picked.length + ' 个' : '导出会话列表 · 全部', 'POST', 'export/sessions', { session: picked.length ? picked.map(s => s.username) : undefined })
      .then(() => setNote({ text: '已加入任务中心。' }), problem => setNote({ text: problem instanceof Error && problem.message ? problem.message : '导出没有开始，请重试。', error: true }));
  };
  return <>
    <p className="card-lead">把{picked.length ? '已选的 ' + picked.length + ' 个会话' : '全部会话'}的列表（会话表的每一行）导出为 JSON。</p>
    <button type="button" className="secondary-button" onClick={start}><DownloadSimple size={14} />导出会话列表</button>
    {note && <p className="card-note" role={note.error ? 'alert' : 'status'}>{note.text}</p>}
  </>;
}
