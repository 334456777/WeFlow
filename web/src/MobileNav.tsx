// 手机上的导航（#335）：底部的标签栏放最常用的几个页面，「更多」从底部打开一个面板，里面是其他页面和连接状态。
// 屏幕更宽时由侧栏代替它。
import { useEffect, useLayoutEffect, useRef, type ComponentType, type ReactNode } from 'react';
import { ArrowLeft, DotsThreeOutline, type IconProps } from '@phosphor-icons/react';
import { usePresence } from './motion';
import { t } from './i18n';

/** 导航里的一个页面。 */
export interface NavItem<Id extends string> {
  id: Id;
  label: string;
  icon: ComponentType<IconProps>;
  /** 完整的名称太长时，标签栏上用的短名称。 */
  short?: string;
}

/** 标签栏和它的「更多」面板。 */
export function MobileNav<Id extends string>({ items, tabs, current, onSelect, status, open, onOpen }: {
  items: readonly NavItem<Id>[];
  /** 直接放在标签栏上的页面；其他页面在「更多」里。 */
  tabs: readonly Id[];
  current: Id;
  onSelect(id: Id): void;
  /** 显示在面板里的连接状态。 */
  status: ReactNode;
  open: boolean;
  onOpen(open: boolean): void;
}) {
  const more = items.filter(item => !tabs.includes(item.id));
  const chosen = more.find(item => item.id === current);
  return <>
    <nav className="mobile-nav" aria-label={t('主导航')}>
      {tabs.map(id => {
        const item = items.find(i => i.id === id)!;
        const Icon = item.icon;
        const selected = id === current;
        return <button key={id} type="button" className="mobile-tab as-row" aria-current={selected ? 'page' : undefined} onClick={() => onSelect(id)}>
          <Icon size={22} weight={selected ? 'fill' : 'regular'} aria-hidden="true" /><span>{t(item.short ?? item.label)}</span>
        </button>;
      })}
      <button type="button" className="mobile-tab as-row" aria-current={chosen ? 'page' : undefined} aria-haspopup="dialog" aria-expanded={open} onClick={() => onOpen(!open)}>
        <DotsThreeOutline size={22} weight={chosen ? 'fill' : 'regular'} aria-hidden="true" /><span>{chosen ? t(chosen.short ?? chosen.label) : t('更多')}</span>
      </button>
    </nav>
    <NavSheet items={more} current={current} open={open} status={status} onSelect={id => { onOpen(false); onSelect(id); }} onClose={() => onOpen(false)} />
  </>;
}

function NavSheet<Id extends string>({ items, current, open, status, onSelect, onClose }: {
  items: readonly NavItem<Id>[]; current: Id; open: boolean; status: ReactNode; onSelect(id: Id): void; onClose(): void;
}) {
  const presence = usePresence(open);
  const ref = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    const dialog = ref.current;
    if (presence && dialog && !dialog.open) dialog.showModal();
  }, [presence]);
  useEffect(() => {
    if (!presence) ref.current?.close();
  }, [presence]);
  if (!presence) return null;
  return <dialog ref={ref} className="nav-sheet" aria-label={t('更多页面')} data-state={presence.closing ? 'closing' : 'open'}
    onCancel={event => { event.preventDefault(); onClose(); }} onClick={event => { if (event.target === event.currentTarget) onClose(); }}>
    <div className="nav-sheet-grid">
      {items.map(item => {
        const Icon = item.icon;
        const selected = item.id === current;
        return <button key={item.id} type="button" className={'nav-sheet-item as-option' + (selected ? ' selected' : '')} aria-current={selected ? 'page' : undefined} onClick={() => onSelect(item.id)}>
          <Icon size={24} weight={selected ? 'fill' : 'regular'} aria-hidden="true" /><span>{t(item.label)}</span>
        </button>;
      })}
    </div>
    <div className="nav-sheet-footer">{status}</div>
  </dialog>;
}

/** 手机上页面详情顶部的栏（#335）：返回列表的按钮和正在显示的内容的标题。屏幕更宽时隐藏。 */
export function MobileBar({ label, title, onBack }: { label: string; title: string; onBack(): void }) {
  return <div className="mobile-bar">
    <button type="button" className="icon-button" aria-label={label} title={label} onClick={onBack}><ArrowLeft size={18} /></button>
    <strong className="clamp-1">{title}</strong>
  </div>;
}
