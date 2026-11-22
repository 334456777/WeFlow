// The console card of the reports page (#208): the report shown, saved as a picture or as a page of its own.
import { useEffect, useState } from 'react';
import { CircleNotch, FileHtml, Image as ImageIcon } from '@phosphor-icons/react';
import { safeName, saveFile } from '../files';
import { reportHtml, reportPng } from './reportFiles';
import { t } from '../i18n';

/** What a save said, of the report it was of. */
interface Note { of: string; text: string; error?: boolean }

/**
 * The buttons that save the report shown, as a PNG or as an HTML page: `report` is its key, null while there is none
 * (it is read, failed, or waits for a friend to be picked), and `find` gives its element.
 */
export function SaveReport({ report, find }: { report: string | null; find(): HTMLElement | null }) {
  const [busy, setBusy] = useState<'png' | 'html' | null>(null);
  const [note, setNote] = useState<Note | null>(null);
  // another report shown: what was said of the one before is let go of, not to come back with it
  useEffect(() => { setNote(null); }, [report]);
  const save = async (kind: 'png' | 'html') => {
    if (!report) return;
    const element = find();
    if (!element) { setNote({ of: report, text: t('报告还没有生成好。'), error: true }); return; }
    setBusy(kind); setNote(null);
    try {
      const title = 'WeFlow ' + (element.getAttribute('aria-label') ?? t('报告'));
      const url = kind === 'html'
        ? saveFile(new TextEncoder().encode(reportHtml(element, title)), 'text/html;charset=utf-8', safeName(title) + '.html')
        : saveFile(new Uint8Array(await (await reportPng(element)).arrayBuffer()), 'image/png', safeName(title) + '.png');
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
      setNote({ of: report, text: t('已交给浏览器下载。头像换成了名字的第一个字。') });
    } catch (problem) {
      setNote({ of: report, text: (problem instanceof Error && problem.message) || t('没有保存，请重试。'), error: true });
    } finally {
      setBusy(null);
    }
  };
  // a save still going when another report is shown says nothing under it
  const said = note?.of === report ? note : null;
  const disabled = busy !== null || !report;
  return <div className="excluded-people">
    <button type="button" className="secondary-button" disabled={disabled} onClick={() => save('png')}>
      {busy === 'png' ? <CircleNotch size={12} className="spin" /> : <ImageIcon size={12} />}{t('保存为图片')}</button>
    <button type="button" className="secondary-button" disabled={disabled} onClick={() => save('html')}>
      {busy === 'html' ? <CircleNotch size={12} className="spin" /> : <FileHtml size={12} />}{t('保存为网页')}</button>
    <p className="card-note">{t('保存的是正在显示的报告，颜色跟随当前主题。')}</p>
    {said && <p className={'card-note' + (said.error ? ' setting-warning' : '')} role={said.error ? 'alert' : undefined} aria-live="polite">{said.text}</p>}
  </div>;
}
