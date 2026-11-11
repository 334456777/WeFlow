// The console cards of the contacts page (#205): how many of each kind, and the list exported as a job of the server.
import { useState } from 'react';
import { DownloadSimple } from '@phosphor-icons/react';
import { CONTACT_FORMATS, CONTACT_KINDS, contactKindLabels, countKinds, type Contact, type ContactFormat, type ContactKind } from '../api/contacts';
import { runJob } from '../jobs/useJobs';
import { GridSkeleton } from '../Skeleton';
import { locale, msg, t } from '../i18n';

export function ContactsSummary({ contacts }: { contacts: Contact[] | null }) {
  if (!contacts) return <GridSkeleton label={t('正在读取通讯录')} cells={5} />;
  const counts = countKinds(contacts);
  const remarked = contacts.filter(c => c.type === 'friend' && c.remark).length;
  return <dl className="info-grid">
    {CONTACT_KINDS.map(kind => <div key={kind}><dt>{t(contactKindLabels[kind])}</dt><dd className="num">{t('{n} 个', { n: counts[kind].toLocaleString(locale()) })}</dd></div>)}
    <div><dt>{t('有备注的好友')}</dt><dd className="num">{t('{remarked} 个', { remarked: remarked.toLocaleString(locale()) })}</dd></div>
  </dl>;
}

export const contactFormatLabel = (format: ContactFormat) => CONTACT_FORMATS.find(f => f.value === format)!.label;

export interface ContactsExportOptions {
  format: ContactFormat;
  kinds: ContactKind[];
  /** The avatars on the HTML page (#708); only an HTML export takes them. */
  avatars: boolean;
}

export const DEFAULT_CONTACTS_EXPORT: ContactsExportOptions = { format: 'excel', kinds: [...CONTACT_KINDS], avatars: false };

/** Whether the export carries the avatars: asked for, and on an HTML page. */
const withAvatars = (options: ContactsExportOptions) => options.avatars && options.format === 'html';

export function contactsExportSummary(options: ContactsExportOptions): string {
  const kinds = options.kinds.length === CONTACT_KINDS.length ? t('全部') : options.kinds.map(k => t(contactKindLabels[k])).join(t('、'));
  return kinds + ' · ' + contactFormatLabel(options.format) + (withAvatars(options) ? ' · ' + t('头像') : '');
}

/** The parameters of `export contacts --parsed`: the kinds in the order of the page, none when every kind is chosen;
 *  with the avatars, the page and the avatars packed into a zip, so the task centre downloads both. */
export function contactsExportParams(options: ContactsExportOptions) {
  const kind = CONTACT_KINDS.filter(k => options.kinds.includes(k));
  const avatars = withAvatars(options);
  return {
    format: options.format, parsed: true, kind: kind.length === CONTACT_KINDS.length ? undefined : kind,
    media: avatars ? ['avatar'] : undefined, zip: avatars ? true : undefined,
  };
}

const AVATAR_NOTE = msg('头像放进 media 文件夹，和页面一起打包成 zip；本机没有的联网下载，联系人多时要等一会儿');

/** `export contacts --parsed`, kept on the server until it is saved from the task centre. */
export function ContactsExport({ options, onChange, ready }: { options: ContactsExportOptions; onChange(options: ContactsExportOptions): void; ready: boolean }) {
  const { format, kinds } = options;
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const start = async () => {
    setBusy(true); setNote(null);
    try {
      await runJob(t('导出通讯录 · {summary}', { summary: contactsExportSummary(options) }), 'POST', 'export/contacts', contactsExportParams(options));
      setNote({ text: t('已加入任务中心。') });
    } catch (problem) {
      setNote({ text: (problem instanceof Error && problem.message) || t('导出没有开始，请重试。'), error: true });
    } finally {
      setBusy(false);
    }
  };
  return <>
    <div className="form-row format-row"><span className="field-label">{t('导出格式')}</span>
      <div className="choice-chips" role="group" aria-label={t('导出格式')}>
        {CONTACT_FORMATS.map(f => <button type="button" key={f.value} className="chip" aria-pressed={format === f.value} onClick={() => { onChange({ ...options, format: f.value }); setNote(null); }}>{f.label}</button>)}
      </div>
    </div>
    <div className="form-row format-row"><span className="field-label">{t('导出哪些')}</span>
      <div className="choice-chips" role="group" aria-label={t('导出哪些联系人')}>
        {CONTACT_KINDS.map(k => <button type="button" key={k} className="chip" aria-pressed={kinds.includes(k)}
          onClick={() => { onChange({ ...options, kinds: kinds.includes(k) ? kinds.filter(x => x !== k) : [...kinds, k] }); setNote(null); }}>{t(contactKindLabels[k])}</button>)}
      </div>
    </div>
    {format === 'html' && <div className="form-row format-row"><span className="field-label">{t('附带媒体')}</span>
      <div className="choice-chips" role="group" aria-label={t('附带的媒体')}>
        <button type="button" className="chip" aria-pressed={options.avatars} onClick={() => { onChange({ ...options, avatars: !options.avatars }); setNote(null); }}>{t('头像')}</button>
      </div>
      <small className="field-note">{t(AVATAR_NOTE)}</small>
    </div>}
    <button type="button" className="primary-button large export-pending" disabled={!ready || busy || !kinds.length} onClick={start}><DownloadSimple size={18} />{kinds.length ? t('导出通讯录') : t('至少选一类')}</button>
    {note && <p className="card-note" role={note.error ? 'alert' : 'status'}>{note.text}</p>}
    <p className="card-note">{t('在服务端导出，完成后在任务中心下载。')}</p>
  </>;
}
