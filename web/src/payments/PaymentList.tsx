// The WeChat Pay records chosen (#204), newest first under the day they were paid: the payee, the amount, how it
// was paid and its state, then the other lines of the notice. All are read already; they are drawn a batch at a
// time as the end comes near.
import { useEffect, useRef, useState } from 'react';
import { formatYuan, type Payment } from '../api/pay';
import { Avatar } from '../Avatar';
import { dayOf, formatClock, formatDayHeading } from '../chat/messages';

/** Records drawn at first and added each time the end comes near. */
const BATCH = 60;

const SHOWN = /金额$|方式$|状态$|^(收款方|收款商户|商户名称|商户)$/;

function PaymentCard({ payment }: { payment: Payment }) {
  const lines = [
    ...(payment.method ? [{ label: '方式', value: payment.method }] : []),
    ...(payment.status ? [{ label: '状态', value: payment.status }] : []),
    ...payment.lines.filter(l => !SHOWN.test(l.label)),
  ];
  return <article className="pay-card" aria-label={payment.merchant + (payment.amount === null ? '' : '，' + (payment.refund ? '退款 ' : '') + formatYuan(payment.amount) + ' 元')}>
    <Avatar name={payment.merchant} url={payment.icon} />
    <div className="pay-body">
      <header>
        <span className="pay-copy"><strong className="clamp-1">{payment.merchant}</strong>
          <span className="pay-title clamp-1">{payment.title && payment.title + ' · '}<time className="num" dateTime={new Date(payment.time * 1000).toISOString()}>{formatClock(payment.time)}</time></span></span>
        {payment.amount !== null && <span className={'pay-amount num' + (payment.refund ? ' refund' : '')}>{payment.refund ? '+' : '-'}{formatYuan(payment.amount)}</span>}
      </header>
      {lines.length > 0 && <dl className="pay-lines">
        {lines.map((l, i) => <div key={i}>{l.label && <dt>{l.label}</dt>}<dd>{l.value}</dd></div>)}
      </dl>}
    </div>
  </article>;
}

/** Give it a `key` per filter, so a new one starts from the top. */
export function PaymentList({ payments, scroller, narrowed }: {
  payments: Payment[];
  /** The scrolling pane, to draw more before its end is reached. */
  scroller: React.RefObject<HTMLElement | null>;
  narrowed: boolean;
}) {
  const [shown, setShown] = useState(BATCH);
  const end = useRef<HTMLDivElement>(null);
  const more = shown < payments.length;
  useEffect(() => {
    const target = end.current;
    if (!target || !more || typeof IntersectionObserver === 'undefined') return;
    const observer = new IntersectionObserver(entries => { if (entries.some(e => e.isIntersecting)) setShown(n => n + BATCH); },
      { root: scroller.current, rootMargin: '0px 0px 800px 0px' });
    observer.observe(target);
    return () => observer.disconnect();
  }, [scroller, more, shown]);

  if (!payments.length) return <div className="pane-message"><p>{narrowed ? '没有符合筛选条件的支付记录。' : '本机没有支付记录。'}</p></div>;
  const now = new Date();
  const rows = payments.slice(0, shown);
  return <div className="payments" aria-label="支付记录">
    {rows.map((p, i) => {
      const day = dayOf(p.time);
      return <div key={p.id} className="pay-row">
        {(i === 0 || dayOf(rows[i - 1].time) !== day) && <div className="date-separator">{formatDayHeading(day, now)}</div>}
        <PaymentCard payment={p} />
      </div>;
    })}
    <div ref={end} className="moments-end">
      {more
        ? <button type="button" className="secondary-button" onClick={() => setShown(n => n + BATCH)}>显示更早的记录</button>
        : <p>共 {payments.length.toLocaleString('zh-CN')} 笔</p>}
    </div>
  </div>;
}
