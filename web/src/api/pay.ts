// 微信支付（#204）：微信支付公众号的通知（`biz pay-records`），在服务进程内读取（`/api/`，
// 见 Wiki 的「命令 API」页）；以及网页从通知的文字里读出的内容：金额、支付方式、状态和收款方。
import { get } from './client';
import { locale, t } from '../i18n';
import type * as Contract from './contract.gen';

/**
 * `biz pay-records` 给出的一条通知（契约的类型，#430）。微信的支付凭证卡片只有一行，如「使用零钱支付¥9.99」，
 * 服务端从中取出支付方式和状态（`method`、`status`）；`description` 由带标签的行组成时，
 * 如「付款金额¥28.00」「支付方式零钱」，逐行读取。
 */
export type PayRecord = Contract.PayRecord;

/** 通知描述里的一行：标签和值。 */
export interface PayLine {
  /** 「付款金额」「支付方式」等；没有标签的行为空。 */
  label: string;
  value: string;
}

/** 网页列出的一条微信支付通知：以分为单位的金额、收款方、支付方式和状态。 */
export interface Payment {
  id: string;
  /** 支付的时间，没有时为通知的时间，Unix 秒数。 */
  time: number;
  title: string;
  merchant: string;
  icon?: string;
  /** 以分为单位；通知里没有金额时为 null。 */
  amount: number | null;
  /** 是退回的钱（退款），而不是付出的钱。 */
  refund: boolean;
  method: string;
  status: string;
  lines: PayLine[];
}

/** 每页读取的微信支付公众号消息数。 */
export const PAY_PAGE = 500;

/** `biz pay-records` 读取通知的会话：微信支付公众号的会话和服务通知。 */
export const PAY_SESSIONS: readonly string[] = ['gh_3dfda90e39d6', 'notifymessage'];

/** 从公众号的第 `offset` 条消息起的一页通知，最新的在前。 */
export const getPayRecords = (offset: number, signal?: AbortSignal) =>
  get<Contract.PayRecords>('api/biz/pay-records', { limit: PAY_PAGE, offset }, signal);

// 长的在前，「付款金额」才不会被当成「金额」
const LABELS = ['付款金额', '支付金额', '消费金额', '交易金额', '退款金额', '到账金额', '收款金额', '支付方式', '付款方式', '退款方式', '到账方式', '交易状态', '支付状态', '退款状态', '当前状态', '商户名称', '收款方', '收款商户', '商品详情', '商品', '商户', '金额', '方式', '状态'] // i18n-ignore: 从微信支付消息里读出的标签
  .sort((a, b) => b.length - a.length);
const AMOUNT = /^(付款|支付|消费|交易|退款|到账|收款)?金额$/;
const METHOD = /方式$/;
const STATUS = /状态$/;
const PAYEE = /^(收款方|收款商户|商户名称|商户)$/;

/** 把描述的一行拆成标签和值：先找已知的标签，再找前 8 个字符以内的冒号；都没有时整行是值。 */
function lineOf(text: string): PayLine {
  const label = LABELS.find(l => text.startsWith(l));
  if (label) return { label, value: text.slice(label.length).replace(/^[\s:：]+/, '').trim() };
  const colon = text.search(/[:：]/);
  if (colon > 0 && colon <= 8) return { label: text.slice(0, colon).trim(), value: text.slice(colon + 1).trim() };
  return { label: '', value: text };
}

/** `text` 里第一个金额，以分为单位；金额写成 `¥28.00`、`￥1,280` 或 `28.00元`。没有金额时为 null。 */
export function fenOf(text: string): number | null {
  const match = /[¥￥]\s*(\d[\d,]*(?:\.\d{1,2})?)/.exec(text) ?? /(\d[\d,]*(?:\.\d{1,2})?)\s*元/.exec(text);
  if (!match) return null;
  const [whole, part = ''] = match[1].replaceAll(',', '').split('.');
  return Number(whole) * 100 + Number((part + '00').slice(0, 2));
}

/** 从通知的标题和描述里带标签的行读出的支付信息。 */
export function paymentOf(record: PayRecord): Payment {
  const lines = record.description.split(/\r?\n/).map(l => l.trim()).filter(Boolean).map(lineOf);
  const find = (pattern: RegExp) => lines.find(l => pattern.test(l.label));
  const amountLine = find(AMOUNT);
  const amount = (amountLine && fenOf(amountLine.value)) ?? fenOf(record.title) ?? lines.map(l => fenOf(l.value)).find(v => v !== null) ?? null;
  const refund = Boolean(amountLine?.label.startsWith('退款')) || /退款/.test(record.title); // i18n-ignore: 消息原文
  return {
    id: record.localId + ':' + record.createTime,
    time: record.timestamp || (record.createTime ?? 0),
    title: record.title,
    merchant: find(PAYEE)?.value || record.merchantName || t('微信支付'),
    icon: record.merchantIcon || undefined,
    amount,
    refund,
    // 较早的服务端不给这两个字段
    method: find(METHOD)?.value || record.method || '',
    status: find(STATUS)?.value || record.status || '',
    lines,
  };
}

/** 把以分为单位的金额写成元，如 2800 分写成「28.00」。 */
export const formatYuan = (fen: number) => (fen / 100).toLocaleString(locale(), { minimumFractionDigits: 2, maximumFractionDigits: 2 });

/** 带 ¥ 符号的金额，如「¥28.00」；负数（退回的钱）写成「-¥5.00」。 */
export const formatMoney = (fen: number) => (fen < 0 ? '-' : '') + '¥' + formatYuan(Math.abs(fen));
