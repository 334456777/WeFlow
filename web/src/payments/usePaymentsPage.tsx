// 微信支付页（#204）：全部通知读一次，在页面里按收款方和日期筛选。
// 通知来自服务端，还没有服务端回答时来自示例数据（#879）。
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
import './payments.css';

/** 微信支付页：页面视图和操作控制台的卡片。`active` 表示正在浏览这一页，打开设置时操作控制台仍保留它的卡片。 */
export function usePaymentsPage(shell: Shell, active: boolean): Page {
  const { backend, scope, setMobileDetail } = shell;
  const live = active && shell.records;
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
  const view = <BrowserPanel label={t('微信支付浏览')} detail={shell.mobileDetail} onBack={() => setMobileDetail(false)} back={t('返回商户列表')} title={t('支付记录')} paneLabel={t('支付记录')} pane={pane} detailKey={filter.merchant}
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
