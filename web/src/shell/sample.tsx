// Whether the records shown are the sample's (#879), for the lists and pages to say so where they show them.
import { createContext, useContext } from 'react';
import { t } from '../i18n';

/** True while the sample server answers the page (`Shell.sample`); the App provides it. */
export const SampleContext = createContext(false);

/** A line under a list's tools, or at the top of a page, while the records are the sample's; nothing otherwise. */
export function SampleNote({ text }: { text?: string }) {
  return useContext(SampleContext) ? <p className="session-note">{text ?? t('示例数据 · 连接 weflow serve 后显示真实记录')}</p> : null;
}
