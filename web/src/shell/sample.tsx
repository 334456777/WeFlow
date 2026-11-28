// 显示的记录是否是示例（#879），供列表和页面在显示记录的地方注明。
import { createContext, useContext } from 'react';
import { t } from '../i18n';

/** 示例服务端回答网页期间为 true（`Shell.sample`），由 App 提供。 */
export const SampleContext = createContext(false);

/** 记录是示例时，在列表的工具下面或页面顶部显示的一行字；否则不显示。 */
export function SampleNote({ text }: { text?: string }) {
  return useContext(SampleContext) ? <p className="session-note">{text ?? t('示例数据 · 连接 weflow serve 后显示真实记录')}</p> : null;
}
