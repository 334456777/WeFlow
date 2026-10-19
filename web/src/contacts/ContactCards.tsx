// The console cards of the contacts page (#205): how many of each kind, and the list exported as a job of the server.
import { useState } from 'react';
import { DownloadSimple } from '@phosphor-icons/react';
import { CONTACT_FORMATS, CONTACT_KINDS, contactKindLabels, countKinds, type Contact, type ContactFormat, type ContactKind } from '../api/contacts';
import { runJob } from '../jobs/useJobs';

export function ContactsSummary({ contacts }: { contacts: Contact[] | null }) {
  if (!contacts) return <p className="empty-text" aria-busy="true">正在读取…</p>;
  const counts = countKinds(contacts);
  const remarked = contacts.filter(c => c.type === 'friend' && c.remark).length;
  return <dl className="info-grid">
    {CONTACT_KINDS.map(kind => <div key={kind}><dt>{contactKindLabels[kind]}</dt><dd className="num">{counts[kind].toLocaleString('zh-CN')} 个</dd></div>)}
    <div><dt>有备注的好友</dt><dd className="num">{remarked.toLocaleString('zh-CN')} 个</dd></div>
  </dl>;
}

export const contactFormatLabel = (format: ContactFormat) => CONTACT_FORMATS.find(f => f.value === format)!.label;

export interface ContactsExportOptions {
  format: ContactFormat;
  kinds: ContactKind[];
}

export const DEFAULT_CONTACTS_EXPORT: ContactsExportOptions = { format: 'excel', kinds: [...CONTACT_KINDS] };

export function contactsExportSummary(options: ContactsExportOptions): string {
  const kinds = options.kinds.length === CONTACT_KINDS.length ? '全部' : options.kinds.map(k => contactKindLabels[k]).join('、');
  return kinds + ' · ' + contactFormatLabel(options.format);
}

/** `export contacts --parsed`, kept on the server until it is saved from the task centre. */
export function ContactsExport({ options, onChange, ready }: { options: ContactsExportOptions; onChange(options: ContactsExportOptions): void; ready: boolean }) {
  const { format, kinds } = options;
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const start = async () => {
    setBusy(true); setNote(null);
    try {
      // the kinds in the order of the page, all of them when every kind is chosen
      const kind = CONTACT_KINDS.filter(k => kinds.includes(k));
      await runJob('导出通讯录 · ' + contactsExportSummary(options), 'POST', 'export/contacts',
        { format, parsed: true, kind: kind.length === CONTACT_KINDS.length ? undefined : kind });
      setNote({ text: '已加入任务中心。' });
    } catch (problem) {
      setNote({ text: (problem instanceof Error && problem.message) || '导出没有开始，请重试。', error: true });
    } finally {
      setBusy(false);
    }
  };
  return <>
    <div className="form-row format-row"><span className="field-label">导出格式</span>
      <div className="choice-chips" role="group" aria-label="导出格式">
        {CONTACT_FORMATS.map(f => <button type="button" key={f.value} className="chip" aria-pressed={format === f.value} onClick={() => { onChange({ ...options, format: f.value }); setNote(null); }}>{f.label}</button>)}
      </div>
    </div>
    <div className="form-row format-row"><span className="field-label">导出哪些</span>
      <div className="choice-chips" role="group" aria-label="导出哪些联系人">
        {CONTACT_KINDS.map(k => <button type="button" key={k} className="chip" aria-pressed={kinds.includes(k)}
          onClick={() => { onChange({ ...options, kinds: kinds.includes(k) ? kinds.filter(x => x !== k) : [...kinds, k] }); setNote(null); }}>{contactKindLabels[k]}</button>)}
      </div>
    </div>
    <button type="button" className="primary-button large export-pending" disabled={!ready || busy || !kinds.length} onClick={start}><DownloadSimple size={18} />{kinds.length ? '导出通讯录' : '至少选一类'}</button>
    {note && <p className="card-note" role={note.error ? 'alert' : 'status'}>{note.text}</p>}
    <p className="card-note">在服务端导出，完成后在任务中心下载。</p>
  </>;
}
