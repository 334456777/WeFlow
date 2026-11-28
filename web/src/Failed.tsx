// A pane whose read failed: why, and a button to read it again.
import { ArrowClockwise } from '@phosphor-icons/react';
import { t } from './i18n';

/** A report that could not be made: why, and a button to try again. */
export const Failed = ({ title, message, onRetry }: { title: string; message: string; onRetry(): void }) =>
  <div className="pane-message" role="alert"><strong>{title}</strong><p>{message}</p>
    <button type="button" className="secondary-button" onClick={onRetry}><ArrowClockwise size={12} />{t('重试')}</button></div>;
