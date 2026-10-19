// The console card of the reports page (#208): the report shown, saved as a picture or as a page of its own.
import { useState } from 'react';
import { CircleNotch, FileHtml, Image as ImageIcon } from '@phosphor-icons/react';
import { safeName, saveFile } from '../files';
import { reportHtml, reportPng } from './reportFiles';

export function SaveReport({ find }: { find(): HTMLElement | null }) {
  const [busy, setBusy] = useState<'png' | 'html' | null>(null);
  const [note, setNote] = useState<{ text: string; error?: boolean } | null>(null);
  const save = async (kind: 'png' | 'html') => {
    const report = find();
    if (!report) { setNote({ text: '报告还没有生成好。', error: true }); return; }
    setBusy(kind); setNote(null);
    try {
      const title = 'WeFlow ' + (report.getAttribute('aria-label') ?? '报告');
      const url = kind === 'html'
        ? saveFile(new TextEncoder().encode(reportHtml(report, title)), 'text/html;charset=utf-8', safeName(title) + '.html')
        : saveFile(new Uint8Array(await (await reportPng(report)).arrayBuffer()), 'image/png', safeName(title) + '.png');
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
      setNote({ text: '已交给浏览器下载。头像换成了名字的第一个字。' });
    } catch (problem) {
      setNote({ text: (problem instanceof Error && problem.message) || '没有保存，请重试。', error: true });
    } finally {
      setBusy(null);
    }
  };
  return <div className="excluded-people">
    <button type="button" className="secondary-button" disabled={busy !== null} onClick={() => save('png')}>
      {busy === 'png' ? <CircleNotch size={12} className="spin" /> : <ImageIcon size={12} />}保存为图片</button>
    <button type="button" className="secondary-button" disabled={busy !== null} onClick={() => save('html')}>
      {busy === 'html' ? <CircleNotch size={12} className="spin" /> : <FileHtml size={12} />}保存为网页</button>
    <p className="card-note">保存的是正在显示的报告，颜色跟随当前主题。</p>
    {note && <p className={'card-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>}
  </div>;
}
