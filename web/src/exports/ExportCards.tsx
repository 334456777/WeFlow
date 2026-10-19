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

const NAMES = [{ value: 'group-nickname', label: '群昵称' }, { value: 'remark', label: '备注' }, { value: 'nickname', label: '昵称' }] as const;
type NameChoice = typeof NAMES[number]['value'];

export interface BatchOptions {
  format: MessageFormat;
  from: string;
  to: string;
  names: NameChoice;
  media: boolean;
  compact: boolean;
}

export const DEFAULT_BATCH: BatchOptions = { format: 'html', from: '', to: '', names: 'group-nickname', media: false, compact: false };

/** The parameters of `export messages` for one conversation. */
export function exportParams(session: string, options: BatchOptions) {
  return {
    'session-id': session, format: options.format, start: options.from || undefined, end: options.to || undefined,
    'display-name': options.names === 'group-nickname' ? undefined : options.names,
    media: options.media ? 'all' : undefined,
    'excel-compact': options.format === 'excel' && options.compact ? true : undefined,
  };
}

const labelOf = (format: MessageFormat) => MESSAGE_FORMATS.find(f => f.value === format)!.label;

export function batchSummary(picked: number, options: BatchOptions): string {
  return (picked ? picked + ' 个会话' : '未选会话') + ' · ' + labelOf(options.format);
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
        await runJob('导出 ' + nameOf(session) + ' · ' + labelOf(options.format), 'POST', 'export/messages', exportParams(session.username, options));
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
      <div className="choice-chips" role="group" aria-label="导出格式">
        {MESSAGE_FORMATS.map(f => <button type="button" key={f.value} className="chip" aria-pressed={options.format === f.value} onClick={() => set({ format: f.value })}>{f.label}</button>)}
      </div>
    </div>
    <div className="form-row"><span className="field-label">时间范围</span><DateRange label="批量导出的时间范围" from={options.from} to={options.to} onChange={range => set(range)} /></div>
    <div className="form-row"><span className="field-label">发送者名称</span>
      <Segmented className="format-group" count={NAMES.length} index={NAMES.findIndex(n => n.value === options.names)} role="group" aria-label="发送者名称">
        {NAMES.map(n => <button type="button" key={n.value} aria-pressed={options.names === n.value} onClick={() => set({ names: n.value })}>{n.label}</button>)}
      </Segmented>
    </div>
    <label className="checkbox"><input type="checkbox" aria-label="附带媒体文件" checked={options.media} onChange={event => set({ media: event.target.checked })} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>附带媒体文件 <small>保存在服务端导出文件旁</small></span></label>
    {options.format === 'excel' && <label className="checkbox"><input type="checkbox" aria-label="Excel 精简列" checked={options.compact} onChange={event => set({ compact: event.target.checked })} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>精简列 <small>时间、发送者、类型、内容</small></span></label>}
    <button type="button" className="primary-button large export-pending" disabled={!picked.length || busy} onClick={start}><DownloadSimple size={18} />{picked.length ? '导出 ' + picked.length + ' 个会话' : '先在左侧选择会话'}</button>
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
