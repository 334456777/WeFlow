// 读取失败的面板：失败的原因和重新读取的按钮。
import { ArrowClockwise } from '@phosphor-icons/react';
import { t } from './i18n';

/** 读取失败时显示的面板：标题、原因和重试按钮。 */
export const Failed = ({ title, message, onRetry }: { title: string; message: string; onRetry(): void }) =>
  <div className="pane-message" role="alert"><strong>{title}</strong><p>{message}</p>
    <button type="button" className="secondary-button" onClick={onRetry}><ArrowClockwise size={12} />{t('重试')}</button></div>;
