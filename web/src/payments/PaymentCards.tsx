// The console's cards on the WeChat Pay page (#204): the days to look at, the totals of what is chosen, and its
// export. The payee is picked in the list.
import { useEffect, useState } from 'react';
import { CheckCircle, DownloadSimple, X } from '@phosphor-icons/react';
import { formatMoney, type Payment } from '../api/pay';
import { DateRange } from '../calendar/DateRange';
import { Segmented } from '../Segmented';
import type { ExportFormat, ExportResult } from '../types';
import { exportPayments } from './payExport';
import { totalsOf, type PayFilter } from './usePayments';
import { GridSkeleton } from '../Skeleton';
import { locale, t } from '../i18n';

const FORMATS: ExportFormat[] = ['HTML', 'JSON', 'Excel'];

export function payFilterSummary(filter: PayFilter): string {
  const parts = [filter.merchant, (filter.from || filter.to) && t('按日期')].filter(Boolean);
  return parts.length ? parts.join(' · ') : t('全部');
}

export function PaymentsFilter({ filter, onChange }: { filter: PayFilter; onChange(filter: PayFilter): void }) {
  const narrowed = Boolean(filter.merchant || filter.from || filter.to);
  return <div className="moments-filter">
    <div className="search-date-row"><span className="field-label">{t('时间范围')}</span>
      <DateRange label={t('支付记录的时间范围')} from={filter.from} to={filter.to} onChange={range => onChange({ ...filter, ...range })} /></div>
    {filter.merchant && <p className="card-note">{t('只看「{merchant}」，在左侧列表里换一个商户或选「全部商户」。', { merchant: filter.merchant })}</p>}
    {narrowed && <button type="button" className="text-button moments-clear" onClick={() => onChange({ merchant: null, from: '', to: '' })}>
      <X size={12} />{t('清除筛选')}</button>}
  </div>;
}

export function PaymentsSummary({ payments }: { payments: Payment[] | null }) {
  if (!payments) return <GridSkeleton label={t('正在读取支付记录')} cells={2} />;
  const totals = totalsOf(payments);
  return <dl className="info-grid">
    <div><dt>{t('笔数')}</dt><dd className="num">{t('{count} 笔', { count: totals.count.toLocaleString(locale()) })}</dd></div>
    <div><dt>{t('支出')}</dt><dd className="num">{formatMoney(totals.paid)}</dd></div>
    {totals.refunded > 0 && <div><dt>{t('退款')}</dt><dd className="num">{formatMoney(totals.refunded)}</dd></div>}
    {totals.refunded > 0 && <div><dt>{t('净支出')}</dt><dd className="num">{formatMoney(totals.paid - totals.refunded)}</dd></div>}
    {totals.unknown > 0 && <div><dt>{t('未写金额')}</dt><dd className="num">{t('{unknown} 笔', { unknown: totals.unknown })}</dd></div>}
  </dl>;
}

/** Saves the records chosen; give it a `key` per filter, so a file made for another one is not offered. */
export function PaymentsExport({ payments, filter }: { payments: Payment[] | null; filter: PayFilter }) {
  const [format, setFormat] = useState<ExportFormat>('Excel');
  const [result, setResult] = useState<ExportResult | null>(null);
  const [error, setError] = useState('');
  useEffect(() => () => { if (result) URL.revokeObjectURL(result.downloadUrl); }, [result]);
  const start = () => {
    if (!payments) return;
    setError('');
    try { setResult(exportPayments(payments, filter, format)); } catch (problem) { setResult(null); setError(problem instanceof Error ? problem.message : t('导出失败，请重试。')); }
  };
  return <>
    <p className="card-lead"><span className="clamp-1">{payments ? t('{n} 笔支付记录', { n: payments.length.toLocaleString(locale()) }) : t('支付记录')}</span></p>
    <div className="form-row format-row"><span className="field-label">{t('导出格式')}</span>
      <Segmented className="format-group" count={FORMATS.length} index={FORMATS.indexOf(format)} role="group" aria-label={t('导出格式')}>
        {FORMATS.map(value => <button type="button" key={value} aria-pressed={format === value} onClick={() => { setFormat(value); setResult(null); }}>{value}</button>)}
      </Segmented>
    </div>
    <button type="button" className="primary-button large" disabled={!payments?.length} onClick={start}><DownloadSimple size={18} />{t('导出筛选结果')}</button>
    {result && <p className="card-note" role="status"><CheckCircle size={14} /> {t('已保存 {fileName}，', { fileName: result.fileName })}<a href={result.downloadUrl} download={result.fileName}>{t('再次下载')}</a></p>}
    {error && <p className="card-note" role="alert">{error}</p>}
  </>;
}
