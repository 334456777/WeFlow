// Beside the WeChat Pay records (#204): all of them, or one payee's, with how often and how much was paid there in
// the days chosen; searchable by name.
import { useMemo, useState } from 'react';
import { ArrowClockwise, MagnifyingGlass, Wallet } from '@phosphor-icons/react';
import { formatMoney } from '../api/pay';
import { Avatar } from '../Avatar';
import { VirtualList } from '../VirtualList';
import type { Merchant, PaymentsState } from './usePayments';
import { ListSkeleton } from '../Skeleton';
import { locale, t } from '../i18n';

const ROW_HEIGHT = 58;
const ALL = '\u0000all';

export function PaymentMerchants({ state, merchants, selected, onSelect, onRetry }: {
  state: PaymentsState;
  /** The payees of the days chosen. */
  merchants: Merchant[];
  selected: string | null;
  onSelect(merchant: string | null): void;
  onRetry(): void;
}) {
  const [query, setQuery] = useState('');
  const rows = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (needle) return merchants.filter(m => m.name.toLocaleLowerCase().includes(needle));
    const all: Merchant = { name: ALL, count: 0, spent: 0 };
    for (const m of merchants) { all.count += m.count; all.spent += m.spent; }
    return [all, ...merchants];
  }, [merchants, query]);

  let body;
  if (state.status === 'loading') {
    body = <ListSkeleton label={t('正在读取支付记录')} note={state.read ? t('已读 {n} 条消息', { n: state.read.toLocaleString(locale()) }) : undefined} />;
  } else if (state.status === 'error') {
    body = <div className="session-message" role="alert"><p>{state.message}</p><button type="button" className="secondary-button" onClick={onRetry}><ArrowClockwise size={14} />{t('重试')}</button></div>;
  } else if (!state.payments.length) {
    body = <p className="session-message">{t('本机没有「微信支付」公众号的消息。')}</p>;
  } else if (rows.length <= 1 && !query.trim()) {
    body = <p className="session-message">{t('这段时间没有支付记录。')}</p>;
  } else if (!rows.length) {
    body = <p className="session-message">{t('没有名称匹配的商户。')}</p>;
  } else {
    body = <VirtualList data-roving="vertical" data-roving-items=".conversation-row" className="conversation-list scroll-area" aria-label={t('商户')} items={rows} rowHeight={ROW_HEIGHT} padding={8} track=".conversation-row"
      keyOf={m => m.name} renderRow={m => {
        const id = m.name === ALL ? null : m.name;
        const active = id === selected;
        const name = id ?? t('全部商户');
        return <button type="button" className={'conversation-row' + (active ? ' active' : '')} aria-pressed={active} aria-label={id ? t('只看{name}，{n} 笔', { name, n: m.count }) : t('全部商户，{n} 笔', { n: m.count })} onClick={() => onSelect(id)}>
          {id ? <Avatar name={name} url={m.icon} /> : <span className="avatar avatar-initial" aria-hidden="true"><Wallet size={20} /></span>}
          <span className="conversation-copy">
            <span className="conversation-head"><span className="conversation-name">{name}</span><span className="conversation-time num">{formatMoney(m.spent)}</span></span>
            <span className="conversation-preview clamp-1">{t('{count} 笔', { count: m.count })}</span>
          </span>
        </button>;
      }} />;
  }

  return <div className="session-pane">
    <div className="session-tools">
      <div className="search-input session-search"><MagnifyingGlass size={16} aria-hidden="true" />
        <input type="search" aria-label={t('搜索商户')} placeholder={t('搜索商户')} value={query} onChange={event => setQuery(event.target.value)} />
      </div>
    </div>
    {body}
  </div>;
}
