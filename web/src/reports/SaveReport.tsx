// 报告页的操作控制台卡片（#208）：把正在显示的报告保存成图片或者单独的网页。
import { useEffect, useState } from 'react';
import { CircleNotch, FileHtml, Image as ImageIcon } from '@phosphor-icons/react';
import { safeName, saveFile } from '../files';
import { reportHtml, reportPng } from './reportFiles';
import { t } from '../i18n';

/** 一次保存的结果提示，以及它属于哪一份报告。 */
interface Note { of: string; text: string; error?: boolean }

/**
 * 把正在显示的报告保存成 PNG 或 HTML 网页的按钮：`report` 是这份报告的键，没有报告（正在读取、读取失败，
 * 或者还在等选好友）时为 null；`find` 返回报告的元素。
 */
export function SaveReport({ report, find }: { report: string | null; find(): HTMLElement | null }) {
  const [busy, setBusy] = useState<'png' | 'html' | null>(null);
  const [note, setNote] = useState<Note | null>(null);
  // 换了报告时清掉对上一份报告的提示，免得换回来时它又出现
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
  // 换了报告后，还在进行的保存不在新的报告下面显示提示
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
