// 导出页的卡片（#211）：已选会话的导出方式，每个会话用 `export messages` 运行一个作业；
// 以及用 `export sessions` 导出会话列表本身。作业都列在任务中心。
import { useState } from 'react';
import { Check, DownloadSimple } from '@phosphor-icons/react';
import type { ChatSession } from '../api/chat';
import { DateRange } from '../calendar/DateRange';
import { Chip } from '../Chip';
import { runJob } from '../jobs/useJobs';
import { Segmented } from '../Segmented';
import { nameOf } from './ExportPicker';
import { msg, t } from '../i18n';

/** `export messages --format` 的格式和页面上显示的名称。 */
export const MESSAGE_FORMATS = [
  { value: 'html', label: 'HTML' }, { value: 'txt', label: 'TXT' }, { value: 'excel', label: 'Excel' },
  { value: 'json', label: 'JSON' }, { value: 'arkme-json', label: 'Arkme JSON' }, { value: 'chatlab', label: 'ChatLab' },
  { value: 'chatlab-jsonl', label: 'ChatLab JSONL' }, { value: 'weclone', label: 'WeClone' }, { value: 'sql', label: 'SQL' },
] as const;
/** `export messages` 的一种格式。 */
export type MessageFormat = typeof MESSAGE_FORMATS[number]['value'];

/** 写出的文件扩展名相同的格式，一次导出只能选其中一种。 */
const CLASHES: readonly MessageFormat[][] = [['json', 'arkme-json']];

/** 按下 `value` 后选中的格式（#342）：已选的取消选择；没选的按列表的顺序加上，同时取消写出相同扩展名的那一种。 */
export function toggleFormat(formats: readonly MessageFormat[], value: MessageFormat): MessageFormat[] {
  // 最后一个也取消：导出随后等待选择格式（#353）
  if (formats.includes(value)) return formats.filter(f => f !== value);
  const clash = CLASHES.find(c => c.includes(value)) ?? [];
  const next = new Set([...formats.filter(f => !clash.includes(f)), value]);
  return MESSAGE_FORMATS.map(f => f.value).filter(f => next.has(f));
}

/** 导出多个文件或者附带媒体时，服务端把它们打包成一个 zip（`--zip`）。 */
export const packed = (formats: readonly MessageFormat[], media: boolean) => formats.length > 1 || media;

/** 把各格式的名称连成一串。 */
export const formatsLabel = (formats: readonly MessageFormat[]) => formats.map(formatLabel).join(t('、'));

/** 格式的芯片，可以多选（#342）。 */
export function FormatChips({ formats, onChange }: { formats: readonly MessageFormat[]; onChange(formats: MessageFormat[]): void }) {
  return <div className="choice-chips" role="group" aria-label={t('导出格式（可多选）')}>
    {MESSAGE_FORMATS.map(f => <Chip key={f.value} label={f.label} pressed={formats.includes(f.value)} onClick={() => onChange(toggleFormat(formats, f.value))} />)}
  </div>;
}

/** `export messages --media` 复制的媒体种类和页面上的名称；名称显示时经过 `t()`。 */
export const MEDIA_KINDS = [
  { value: 'image', label: msg('图片') }, { value: 'voice', label: msg('语音') }, { value: 'video', label: msg('视频') },
  { value: 'emoji', label: msg('表情') }, { value: 'file', label: msg('文件') }, { value: 'avatar', label: msg('头像') },
] as const;
/** `export messages --media` 复制的一种媒体。 */
export type MediaKind = typeof MEDIA_KINDS[number]['value'];

/** 按下 `value` 后选中的媒体：已选的取消，没选的按列表的顺序加上。 */
export function toggleMedia(kinds: readonly MediaKind[], value: MediaKind): MediaKind[] {
  const next = new Set(kinds.includes(value) ? kinds.filter(k => k !== value) : [...kinds, value]);
  return MEDIA_KINDS.map(k => k.value).filter(k => next.has(k));
}

/** 用页面的语言把各媒体的名称连成一串。 */
export const mediaLabel = (kinds: readonly MediaKind[]) => kinds.map(k => t(MEDIA_KINDS.find(m => m.value === k)!.label)).join(t('、'));

/** 媒体的芯片：与格式一样可以多选，也可以一个都不选。 */
export function MediaChips({ kinds, onChange, label }: { kinds: readonly MediaKind[]; onChange(kinds: MediaKind[]): void; label: string }) {
  return <div className="choice-chips" role="group" aria-label={label}>
    {MEDIA_KINDS.map(k => <Chip key={k.value} label={t(k.label)} pressed={kinds.includes(k.value)} onClick={() => onChange(toggleMedia(kinds, k.value))} />)}
  </div>;
}

const NAMES = [{ value: 'group-nickname', label: msg('群昵称') }, { value: 'remark', label: msg('备注') }, { value: 'nickname', label: msg('昵称') }] as const;
type NameChoice = typeof NAMES[number]['value'];

/** 已选会话的导出选项：格式、日期范围、发送者名称、附带的媒体和 Excel 的精简列。 */
export interface BatchOptions {
  formats: MessageFormat[];
  from: string;
  to: string;
  names: NameChoice;
  media: MediaKind[];
  compact: boolean;
}

/** 默认的导出选项：HTML，不限日期，发送者用群昵称，不附带媒体。 */
export const DEFAULT_BATCH: BatchOptions = { formats: ['html'], from: '', to: '', names: 'group-nickname', media: [], compact: false };

/** 一个会话的 `export messages` 参数。 */
export function exportParams(session: string, options: BatchOptions) {
  return {
    'session-id': session, format: options.formats, start: options.from || undefined, end: options.to || undefined,
    'display-name': options.names === 'group-nickname' ? undefined : options.names,
    media: options.media.length ? options.media : undefined,
    'excel-compact': options.formats.includes('excel') && options.compact ? true : undefined,
    zip: packed(options.formats, options.media.length > 0) ? true : undefined,
  };
}

/** 格式的名称。 */
export const formatLabel = (format: MessageFormat) => MESSAGE_FORMATS.find(f => f.value === format)!.label;

/** 卡片收起时标题栏上的批量导出摘要：会话数、格式和媒体。 */
export function batchSummary(picked: number, options: BatchOptions): string {
  return (picked ? t('{n} 个会话', { n: picked }) : t('未选会话')) + ' · ' + formatsLabel(options.formats) + (options.media.length ? ' · ' + t('含{kinds}', { kinds: mediaLabel(options.media) }) : '');
}

/** 设置已选会话怎样导出的卡片。 */
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
      // 每个会话一个作业，按列表的顺序加入；服务端依次运行
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
    <div className="form-row format-row chips-below"><span className="field-label">{t('导出格式')}</span>
      <FormatChips formats={options.formats} onChange={formats => set({ formats })} />
    </div>
    <div className="form-row"><span className="field-label">{t('时间范围')}</span><DateRange label={t('批量导出的时间范围')} from={options.from} to={options.to} onChange={range => set(range)} /></div>
    <div className="form-row"><span className="field-label">{t('发送者名称')}</span>
      <Segmented className="format-group" count={NAMES.length} index={NAMES.findIndex(n => n.value === options.names)} role="group" aria-label={t('发送者名称')}>
        {NAMES.map(n => <button type="button" key={n.value} aria-pressed={options.names === n.value} onClick={() => set({ names: n.value })}>{t(n.label)}</button>)}
      </Segmented>
    </div>
    <div className="form-row format-row chips-below"><span className="field-label">{t('附带媒体')}</span>
      <MediaChips kinds={options.media} label={t('附带的媒体（可多选）')} onChange={media => set({ media })} />
    </div>
    {options.formats.includes('excel') && <label className="checkbox"><input type="checkbox" aria-label={t('Excel 精简列')} checked={options.compact} onChange={event => set({ compact: event.target.checked })} /><span className="checkbox-box" aria-hidden="true"><Check size={12} weight="bold" /></span><span>{t('精简列')} <small>{t('时间、发送者、类型、内容')}</small></span></label>}
    <button type="button" className="primary-button large export-pending" disabled={!options.formats.length || !picked.length || busy} onClick={start}><DownloadSimple size={18} />{!options.formats.length ? t('选择导出格式') : picked.length ? t('导出 {n} 个会话', { n: picked.length }) : t('先在左侧选择会话')}</button>
    {note && <p className="card-note" role={note.error ? 'alert' : 'status'}>{note.text}</p>}
    <p className="card-note">{t('每个会话一个任务，在服务端依次运行；完成后在任务中心下载。')}</p>
  </>;
}

/** `export sessions`：导出已选会话的列表，没有选时导出全部会话的列表。 */
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
