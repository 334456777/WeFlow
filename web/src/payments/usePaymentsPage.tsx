// The WeChat Pay page (#204): all the notices read once, narrowed by payee and days here. Not connected, the sample
// is shown in the chat page's browser instead.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { Page, Shell } from '../shell/page';
import { BrowserPanel } from '../shell/panels';
import { PaymentsSkeleton } from '../Skeleton';
import { TokenPrompt } from '../TokenForm';
import { payFilterSummary, PaymentsExport, PaymentsFilter, PaymentsSummary } from './PaymentCards';
import { PaymentList } from './PaymentList';
import { PaymentMerchants } from './PaymentMerchants';
import { ALL_PAYMENTS, merchantsOf, usePayments, withinDays, type PayFilter } from './usePayments';
import { t } from '../i18n';

/** `active`: the page browsed, the one the console keeps the cards of while the settings are open. */
export function usePaymentsPage(shell: Shell, active: boolean): Page {
  const { backend, scope, setMobileDetail } = shell;
  const live = active && shell.connected;
  const [filter, setFilter] = useState<PayFilter>(ALL_PAYMENTS);
  useEffect(() => { setFilter(ALL_PAYMENTS); }, [scope]);
  const [state, reload] = usePayments(live, scope);
  const days = useMemo(() => state.status === 'ready' ? withinDays(state.payments, filter) : null, [state, filter]);
  const merchants = useMemo(() => days ? merchantsOf(days) : [], [days]);
  const chosen = useMemo(() => days && (filter.merchant === null ? days : days.filter(p => p.merchant === filter.merchant)), [days, filter.merchant]);
  const pane = useRef<HTMLDivElement>(null);
  const cards = [
    {
      id: 'pay-filter', title: t('筛选支付记录'), summary: payFilterSummary(filter),
      content: <PaymentsFilter filter={filter} onChange={setFilter} />,
    },
    {
      id: 'pay-summary', title: t('合计'), summary: chosen ? t('{n} 笔', { n: chosen.length }) : '',
      content: <PaymentsSummary payments={chosen} />,
    },
    {
      id: 'pay-export', title: t('导出支付记录'),
      content: <PaymentsExport key={scope + ':' + JSON.stringify(filter)} payments={chosen} filter={filter} />,
    },
  ];
  if (!live && !(active && backend.state === 'unauthorized')) return { cards };
  const view = <BrowserPanel label={t('微信支付浏览')} detail={shell.mobileDetail} onBack={() => setMobileDetail(false)} back={t('返回商户列表')} title={t('支付记录')} paneLabel={t('支付记录')} pane={pane}
    list={backend.state === 'unauthorized'
      ? <TokenPrompt />
      : <PaymentMerchants state={state} merchants={merchants} selected={filter.merchant} onRetry={reload}
        onSelect={merchant => { setFilter(f => ({ ...f, merchant })); setMobileDetail(true); }} />}>
    {live && (chosen
      ? <PaymentList key={scope + ':' + JSON.stringify(filter)} payments={chosen} scroller={pane} narrowed={Boolean(filter.merchant || filter.from || filter.to)} />
      : state.status === 'loading' && <PaymentsSkeleton label={t('正在读取支付记录')} />)}
  </BrowserPanel>;
  return { view, cards, shown: live ? cards.map(card => card.id) : undefined };
}
