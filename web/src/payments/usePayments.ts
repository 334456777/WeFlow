// 全部微信支付通知（#204）：每个服务端或账号（scope.ts）一页接一页地读一遍，实时消息（#201）可能带来新通知时再读一遍。
// `biz pay-records` 没有筛选参数，所以按日期和收款方筛选由页面自己做。
import { useEffect, useRef, useState } from 'react';
import { ApiError } from '../api/client';
import { useGathered, useLive } from '../api/live';
import { t } from '../i18n';
import { getPayRecords, paymentOf, PAY_PAGE, PAY_SESSIONS, type Payment } from '../api/pay';
import { dayOf } from '../format';

/** 支付通知的读取状态：读取中（已读到多少条）、已读完（全部通知，最新的在前），或者失败的原因。 */
export type PaymentsState =
  | { status: 'loading'; read: number }
  | { status: 'ready'; payments: Payment[] }
  | { status: 'error'; message: string };

/** `enabled` 之后读取的全部微信支付通知，以及重新读取的函数；同一作用域重读时保留已显示的记录。 */
export function usePayments(enabled: boolean, scope: number): [PaymentsState, () => void] {
  const [state, setState] = useState<{ scope: number; value: PaymentsState } | null>(null);
  const [round, setRound] = useState(0);
  // 只要服务端和账号没变、这期间也不可能来了新通知，离开页面再回来时保留已读到的记录
  const read = useRef<string | null>(null);
  const refresh = useGathered(() => setRound(n => n + 1));
  useLive(event => {
    if (event.type === 'resumed' || PAY_SESSIONS.includes(event.sessionId)) refresh();
  });
  useEffect(() => {
    const key = scope + ':' + round;
    if (!enabled || read.current === key) return;
    read.current = key;
    const controller = new AbortController();
    const { signal } = controller;
    let done = false;
    setState(old => (old?.scope === scope && old.value.status === 'ready' ? old : { scope, value: { status: 'loading', read: 0 } }));
    (async () => {
      const payments: Payment[] = [];
      const seen = new Set<string>();
      for (let offset = 0; ; offset += PAY_PAGE) {
        const page = await getPayRecords(offset, signal);
        for (const record of page.records) {
          const payment = paymentOf(record);
          if (!seen.has(payment.id)) { seen.add(payment.id); payments.push(payment); }
        }
        if (!page.hasMore) break;
        setState(old => (old?.scope === scope && old.value.status === 'ready' ? old : { scope, value: { status: 'loading', read: payments.length } }));
      }
      // 按支付的时间（而不是通知的时间）排列，最新的在前
      payments.sort((a, b) => b.time - a.time);
      return payments;
    })().then(
      payments => {
        if (signal.aborted) return;
        done = true;
        setState({ scope, value: { status: 'ready', payments } });
      },
      problem => {
        if (signal.aborted) return;
        done = true;
        read.current = null;
        setState({ scope, value: { status: 'error', message: (problem instanceof ApiError && problem.message) || t('读取支付记录失败') } });
      },
    );
    // 还没读完就离开了页面：下次打开时重新读取
    return () => {
      if (done) return;
      controller.abort();
      if (read.current === key) read.current = null;
    };
  }, [enabled, scope, round]);
  return [state?.scope === scope ? state.value : { status: 'loading', read: 0 }, () => setRound(n => n + 1)];
}

/** 页面筛选支付记录的条件。 */
export interface PayFilter {
  /** 一个收款方，或者全部（null）。 */
  merchant: string | null;
  /** 日期，写成 `2026-10-12`；为空表示不限。 */
  from: string;
  to: string;
}

/** 不筛选：全部支付记录。 */
export const ALL_PAYMENTS: PayFilter = { merchant: null, from: '', to: '' };

/** 落在筛选的日期范围内的支付记录。 */
export function withinDays(payments: Payment[], { from, to }: Pick<PayFilter, 'from' | 'to'>): Payment[] {
  if (!from && !to) return payments;
  return payments.filter(p => {
    const day = dayOf(p.time);
    return (!from || day >= from) && (!to || day <= to);
  });
}

/** 符合筛选的日期范围和收款方的支付记录。 */
export const matches = (payments: Payment[], filter: PayFilter) =>
  withinDays(payments, filter).filter(p => filter.merchant === null || p.merchant === filter.merchant);

/** 一个收款方，以及在那里支付的次数和金额。 */
export interface Merchant {
  name: string;
  icon?: string;
  count: number;
  /** 支付的金额（分），减去了退款。 */
  spent: number;
}

/** `payments` 里的收款方，支付金额多的在前；金额相同时按次数，再按名称排。 */
export function merchantsOf(payments: Payment[]): Merchant[] {
  const byName = new Map<string, Merchant>();
  for (const p of payments) {
    const m = byName.get(p.merchant) ?? { name: p.merchant, icon: p.icon, count: 0, spent: 0 };
    m.count += 1;
    m.spent += (p.amount ?? 0) * (p.refund ? -1 : 1);
    m.icon ??= p.icon;
    byName.set(p.merchant, m);
  }
  return [...byName.values()].sort((a, b) => b.spent - a.spent || b.count - a.count || a.name.localeCompare(b.name, 'zh-CN'));
}

/** 一组支付记录的合计。 */
export interface PayTotals {
  count: number;
  /** 支付的金额和退款的金额（分）。 */
  paid: number;
  refunded: number;
  /** 没有写金额的通知条数。 */
  unknown: number;
}

/** `payments` 的合计：条数、支付的金额（分）、退款的金额（分），以及没有写金额的条数。 */
export function totalsOf(payments: Payment[]): PayTotals {
  const totals: PayTotals = { count: payments.length, paid: 0, refunded: 0, unknown: 0 };
  for (const p of payments) {
    if (p.amount === null) totals.unknown += 1;
    else if (p.refund) totals.refunded += p.amount;
    else totals.paid += p.amount;
  }
  return totals;
}
