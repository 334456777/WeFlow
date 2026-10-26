// WeChat Pay (#204): the notices of the WeChat Pay account (`biz pay-records`), read in the server's process
// (`/api/`, docs/serve-api.md), and what the page makes of their text: the amount, how it was paid, its state, the payee.
import { get } from './client';

/** A notice as `biz pay-records` gives it. */
export interface PayRecord {
  localId: number;
  createTime: number;
  title: string;
  /** The notice's lines, such as 「付款金额¥28.00」 and 「支付方式零钱」. */
  description: string;
  merchantName: string;
  merchantIcon: string;
  /** Unix seconds of the payment, 0 when the notice has none. */
  timestamp: number;
}

export interface PayLine {
  /** 「付款金额」, 「支付方式」, …; empty for a line without one. */
  label: string;
  value: string;
}

export interface Payment {
  id: string;
  /** Unix seconds: of the payment, or of the notice. */
  time: number;
  title: string;
  merchant: string;
  icon?: string;
  /** In fen (分); null when the notice names no amount. */
  amount: number | null;
  /** Money back (a refund), rather than money paid. */
  refund: boolean;
  method: string;
  status: string;
  lines: PayLine[];
}

/** Messages of the WeChat Pay account per page. */
export const PAY_PAGE = 500;

/** A page of notices, newest first, from the `offset`-th message of the account on. */
export async function getPayRecords(offset: number, signal?: AbortSignal): Promise<{ records: PayRecord[]; hasMore: boolean }> {
  const data = await get<{ records?: PayRecord[]; hasMore?: boolean }>('api/biz/pay-records', { limit: PAY_PAGE, offset }, signal);
  const records = data?.records ?? [];
  // a server before rc.6 does not say; a full page may have more
  return { records, hasMore: data?.hasMore ?? records.length >= PAY_PAGE };
}

// longest first, so 「付款金额」 is not read as 「金额」
const LABELS = ['付款金额', '支付金额', '消费金额', '交易金额', '退款金额', '到账金额', '收款金额', '支付方式', '付款方式', '退款方式', '到账方式', '交易状态', '支付状态', '退款状态', '当前状态', '商户名称', '收款方', '收款商户', '商品详情', '商品', '商户', '金额', '方式', '状态']
  .sort((a, b) => b.length - a.length);
const AMOUNT = /^(付款|支付|消费|交易|退款|到账|收款)?金额$/;
const METHOD = /方式$/;
const STATUS = /状态$/;
const PAYEE = /^(收款方|收款商户|商户名称|商户)$/;

function lineOf(text: string): PayLine {
  const label = LABELS.find(l => text.startsWith(l));
  if (label) return { label, value: text.slice(label.length).replace(/^[\s:：]+/, '').trim() };
  const colon = text.search(/[:：]/);
  if (colon > 0 && colon <= 8) return { label: text.slice(0, colon).trim(), value: text.slice(colon + 1).trim() };
  return { label: '', value: text };
}

/** Fen of the first amount in `text`: 「¥28.00」, 「￥1,280」 or 「28.00元」. */
export function fenOf(text: string): number | null {
  const match = /[¥￥]\s*(\d[\d,]*(?:\.\d{1,2})?)/.exec(text) ?? /(\d[\d,]*(?:\.\d{1,2})?)\s*元/.exec(text);
  if (!match) return null;
  const [whole, part = ''] = match[1].replaceAll(',', '').split('.');
  return Number(whole) * 100 + Number((part + '00').slice(0, 2));
}

export function paymentOf(record: PayRecord): Payment {
  const lines = record.description.split(/\r?\n/).map(l => l.trim()).filter(Boolean).map(lineOf);
  const find = (pattern: RegExp) => lines.find(l => pattern.test(l.label));
  const amountLine = find(AMOUNT);
  const amount = (amountLine && fenOf(amountLine.value)) ?? fenOf(record.title) ?? lines.map(l => fenOf(l.value)).find(v => v !== null) ?? null;
  const refund = Boolean(amountLine?.label.startsWith('退款')) || /退款/.test(record.title);
  return {
    id: record.localId + ':' + record.createTime,
    time: record.timestamp || record.createTime,
    title: record.title,
    merchant: find(PAYEE)?.value || record.merchantName || '微信支付',
    icon: record.merchantIcon || undefined,
    amount,
    refund,
    method: find(METHOD)?.value ?? '',
    status: find(STATUS)?.value ?? '',
    lines,
  };
}

/** 「28.00」 of 2800 fen. */
export const formatYuan = (fen: number) => (fen / 100).toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 });

/** 「¥28.00」, or 「-¥5.00」 for money that came back. */
export const formatMoney = (fen: number) => (fen < 0 ? '-' : '') + '¥' + formatYuan(Math.abs(fen));
