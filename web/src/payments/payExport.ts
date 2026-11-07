// The WeChat Pay records chosen (#204), saved by the browser as HTML, JSON or Excel, as the chat export offers.
import { strToU8 } from 'fflate';
import { formatYuan, type Payment } from '../api/pay';
import { dayOf, formatClock } from '../chat/messages';
import { makeSheet, safeName, saveFile, xmlEscape, XLSX_TYPE } from '../files';
import { locale, msg, t } from '../i18n';
import type { ExportFormat, ExportResult } from '../types';
import { totalsOf, type PayFilter } from './usePayments';

// shown through t(): the file is in the page's language
const COLUMNS = [msg('日期'), msg('时间'), msg('商户'), msg('收支'), msg('金额（元）'), msg('方式'), msg('状态'), msg('标题'), msg('详情')];
const columns = () => COLUMNS.map(c => t(c));

const rowOf = (p: Payment) => [
  dayOf(p.time), formatClock(p.time), p.merchant, p.refund ? t('退款') : t('支出'), p.amount === null ? '' : formatYuan(p.amount).replaceAll(',', ''),
  p.method, p.status, p.title, p.lines.map(l => (l.label ? l.label + '：' : '') + l.value).join('\n'),
];

/** 「全部商户_2026-10-01_2026-10-12」, for the file's name and heading. */
export function describe(filter: PayFilter): string {
  const days = filter.from || filter.to ? (filter.from || t('最早')) + '_' + (filter.to || t('最近')) : t('全部时间');
  return (filter.merchant ?? t('全部商户')) + '_' + days;
}

export function exportPayments(payments: Payment[], filter: PayFilter, format: ExportFormat): ExportResult {
  if (!payments.length) throw new Error(t('筛选范围内没有支付记录。'));
  const name = safeName(t('微信支付') + '_' + describe(filter));
  const totals = totalsOf(payments);
  let bytes: Uint8Array;
  let fileName: string;
  let type: string;
  if (format === 'JSON') {
    bytes = strToU8(JSON.stringify({
      source: 'WeFlow', kind: '微信支付', merchant: filter.merchant, from: filter.from || null, to: filter.to || null, // i18n-ignore: a field of the file
      count: totals.count, paidFen: totals.paid, refundedFen: totals.refunded,
      records: payments.map(p => ({ time: p.time, merchant: p.merchant, title: p.title, amountFen: p.amount, refund: p.refund, method: p.method, status: p.status, lines: p.lines })),
    }, null, 2));
    fileName = name + '.json'; type = 'application/json;charset=utf-8';
  } else if (format === 'Excel') {
    bytes = makeSheet(t('微信支付'), [columns(), ...payments.map(rowOf)], [12, 8, 24, 6, 12, 14, 14, 24, 48]);
    fileName = name + '.xlsx'; type = XLSX_TYPE;
  } else {
    const cell = (value: string) => '<td>' + xmlEscape(value).replaceAll('\n', '<br>') + '</td>';
    const rows = payments.map(p => '<tr' + (p.refund ? ' class="refund"' : '') + '>' + rowOf(p).map(cell).join('') + '</tr>').join('');
    const summary = t('{n} 笔 · 支出 ¥{paid}', { n: totals.count, paid: formatYuan(totals.paid) }) + (totals.refunded ? ' · ' + t('退款 ¥{refunded}', { refunded: formatYuan(totals.refunded) }) : '');
    const title = xmlEscape(t('微信支付记录'));
    bytes = strToU8('<!doctype html><html lang="' + locale() + '"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>' + title + '</title>'
      + '<style>body{font:15px/1.6 system-ui,sans-serif;max-width:1100px;margin:36px auto;padding:0 24px;color:#202326;background:#fafbfc}header{border-bottom:1px solid #e1e5e9;margin-bottom:24px}table{width:100%;border-collapse:collapse;background:white}th,td{padding:8px 10px;border-bottom:1px solid #e1e5e9;text-align:left;vertical-align:top}th{position:sticky;top:0;background:#eceef0;font-weight:600}td:nth-child(5){text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}.refund td:nth-child(5){color:#1a7f37}td:last-child{color:#5b6270;font-size:13px}</style>'
      + '<header><h1>' + title + '</h1><p>' + xmlEscape(describe(filter).replaceAll('_', ' · ')) + ' · ' + xmlEscape(summary) + '</p></header>'
      + '<table><thead><tr>' + columns().map(c => '<th>' + xmlEscape(c) + '</th>').join('') + '</tr></thead><tbody>' + rows + '</tbody></table></html>');
    fileName = name + '.html'; type = 'text/html;charset=utf-8';
  }
  return { fileName, count: payments.length, downloadUrl: saveFile(bytes, type, fileName) };
}
