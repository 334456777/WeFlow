// All the WeChat Pay notices (#204), read a page after another once per server or account (scope.ts): the page
// narrows them by days and payee itself, as `biz pay-records` has no filters.
import { useEffect, useState } from 'react';
import { ApiError } from '../api/client';
import { getPayRecords, paymentOf, PAY_PAGE, type Payment } from '../api/pay';
import { dayOf } from '../chat/messages';

export type PaymentsState =
  | { status: 'loading'; read: number }
  | { status: 'ready'; payments: Payment[] }
  | { status: 'error'; message: string };

export function usePayments(enabled: boolean, scope: number): [PaymentsState, () => void] {
  const [state, setState] = useState<{ scope: number; value: PaymentsState } | null>(null);
  const [round, setRound] = useState(0);
  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    const { signal } = controller;
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
      // newest first, by the time of the payment rather than of the notice
      payments.sort((a, b) => b.time - a.time);
      return payments;
    })().then(
      payments => { if (!signal.aborted) setState({ scope, value: { status: 'ready', payments } }); },
      problem => {
        if (signal.aborted) return;
        setState({ scope, value: { status: 'error', message: (problem instanceof ApiError && problem.message) || '读取支付记录失败' } });
      },
    );
    return () => controller.abort();
  }, [enabled, scope, round]);
  return [state?.scope === scope ? state.value : { status: 'loading', read: 0 }, () => setRound(n => n + 1)];
}

export interface PayFilter {
  /** One payee, or all of them. */
  merchant: string | null;
  /** Days as `2026-10-12`, empty for no limit. */
  from: string;
  to: string;
}

export const ALL_PAYMENTS: PayFilter = { merchant: null, from: '', to: '' };

/** The payments of the filter's days. */
export function withinDays(payments: Payment[], { from, to }: Pick<PayFilter, 'from' | 'to'>): Payment[] {
  if (!from && !to) return payments;
  return payments.filter(p => {
    const day = dayOf(p.time);
    return (!from || day >= from) && (!to || day <= to);
  });
}

export const matches = (payments: Payment[], filter: PayFilter) =>
  withinDays(payments, filter).filter(p => filter.merchant === null || p.merchant === filter.merchant);

export interface Merchant {
  name: string;
  icon?: string;
  count: number;
  /** Fen paid, refunds taken off. */
  spent: number;
}

/** The payees of `payments`, most paid first. */
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

export interface PayTotals {
  count: number;
  /** Fen paid, and fen refunded. */
  paid: number;
  refunded: number;
  /** Notices that name no amount. */
  unknown: number;
}

export function totalsOf(payments: Payment[]): PayTotals {
  const totals: PayTotals = { count: payments.length, paid: 0, refunded: 0, unknown: 0 };
  for (const p of payments) {
    if (p.amount === null) totals.unknown += 1;
    else if (p.refund) totals.refunded += p.amount;
    else totals.paid += p.amount;
  }
  return totals;
}
