// The navigation on a phone (#335): a tab bar along the bottom with the pages used most, and 「更多」, which opens the
// others in a sheet from the bottom, with the connection state. The sidebar takes its place on wider screens.
import { useEffect, useLayoutEffect, useRef, type ComponentType, type ReactNode } from 'react';
import { ArrowLeft, DotsThreeOutline, type IconProps } from '@phosphor-icons/react';
import { usePresence } from './motion';
import { t } from './i18n';

export interface NavItem<Id extends string> {
  id: Id;
  label: string;
  icon: ComponentType<IconProps>;
  /** The tab's own label where the full one is too long. */
  short?: string;
}

export function MobileNav<Id extends string>({ items, tabs, current, onSelect, status, open, onOpen }: {
  items: readonly NavItem<Id>[];
  /** The pages on the bar itself; the others are under 「更多」. */
  tabs: readonly Id[];
  current: Id;
  onSelect(id: Id): void;
  /** The connection state, shown in the sheet. */
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
        return <button key={id} type="button" className="mobile-tab" aria-current={selected ? 'page' : undefined} onClick={() => onSelect(id)}>
          <Icon size={22} weight={selected ? 'fill' : 'regular'} aria-hidden="true" /><span>{t(item.short ?? item.label)}</span>
        </button>;
      })}
      <button type="button" className="mobile-tab" aria-current={chosen ? 'page' : undefined} aria-haspopup="dialog" aria-expanded={open} onClick={() => onOpen(!open)}>
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
        return <button key={item.id} type="button" className={'nav-sheet-item' + (selected ? ' selected' : '')} aria-current={selected ? 'page' : undefined} onClick={() => onSelect(item.id)}>
          <Icon size={24} weight={selected ? 'fill' : 'regular'} aria-hidden="true" /><span>{t(item.label)}</span>
        </button>;
      })}
    </div>
    <div className="nav-sheet-footer">{status}</div>
  </dialog>;
}

/** The bar over a page's detail on a phone (#335): back to its list, and what is shown. Hidden on wider screens. */
export function MobileBar({ label, title, onBack }: { label: string; title: string; onBack(): void }) {
  return <div className="mobile-bar">
    <button type="button" className="icon-button" aria-label={label} title={label} onClick={onBack}><ArrowLeft size={18} /></button>
    <strong className="clamp-1">{title}</strong>
  </div>;
}
