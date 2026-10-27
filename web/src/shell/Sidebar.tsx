// The navigation on the left (#130), folded by its button or on a narrow window; on a phone a tab bar takes its
// place (#335). The connection is shown at its foot.
import { useEffect, useState } from 'react';
import {
  AddressBook, Aperture, Archive, CalendarStar, ChartBar, ChatCircleDots, Export, Footprints, Gear, Images, Lightbulb,
  SidebarSimple, Wallet,
} from '@phosphor-icons/react';
import type { Backend } from '../api/backend';
import { HoverTrack } from '../HoverTrack';
import { MobileNav } from '../MobileNav';
import type { Navigation } from '../types';
import { Connection } from './Connection';
import { msg, t } from '../i18n';

const navigation = [
  { id: 'chat', label: msg('聊天'), icon: ChatCircleDots },
  { id: 'moments', label: msg('朋友圈'), icon: Aperture },
  { id: 'payments', label: msg('微信支付'), icon: Wallet },
  { id: 'contacts', label: msg('通讯录'), icon: AddressBook },
  { id: 'resources', label: msg('资源浏览'), icon: Images, short: msg('资源') },
  { id: 'analytics', label: msg('聊天分析'), icon: ChartBar },
  { id: 'reports', label: msg('年度报告'), icon: CalendarStar },
  { id: 'footprint', label: msg('我的足迹'), icon: Footprints },
  { id: 'insights', label: msg('灵感信箱'), icon: Lightbulb },
  { id: 'export', label: msg('导出'), icon: Export },
  { id: 'backup', label: msg('数据库备份'), icon: Archive },
  { id: 'settings', label: msg('设置'), icon: Gear },
] as const;

/** The pages on a phone's tab bar; the others are under 「更多」 (#335). */
const MOBILE_TABS = ['chat', 'contacts', 'moments', 'resources'] as const;

const SIDEBAR_KEY = 'weflow.sidebar';

function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => Boolean(window.matchMedia?.(query).matches));
  useEffect(() => {
    const list = window.matchMedia?.(query);
    if (!list) return;
    const update = () => setMatches(list.matches);
    update();
    list.addEventListener('change', update);
    return () => list.removeEventListener('change', update);
  }, [query]);
  return matches;
}

export interface SidebarFold {
  /** Folded by its button (remembered) or because the window is narrow. */
  folded: boolean;
  narrow: boolean;
  toggle(): void;
}

export function useSidebarFold(): SidebarFold {
  const narrow = useMediaQuery('(max-width: 1100px)');
  const [chosen, setChosen] = useState(() => { try { return localStorage.getItem(SIDEBAR_KEY) === 'folded'; } catch { return false; } });
  const toggle = () => {
    const next = !chosen;
    setChosen(next);
    try { localStorage.setItem(SIDEBAR_KEY, next ? 'folded' : 'open'); } catch { /* only lost on reload */ }
  };
  return { folded: narrow || chosen, narrow, toggle };
}

export function Sidebar({ section, onSelect, fold, backend, realtime }: {
  section: Navigation; onSelect(section: Navigation): void; fold: SidebarFold; backend: Backend; realtime: boolean;
}) {
  const phone = useMediaQuery('(max-width: 640px)');
  const [sheet, setSheet] = useState(false);
  const connection = <Connection backend={backend} realtime={realtime} />;
  return <>
    <aside className="sidebar">
      <nav aria-label={t('主导航')} data-roving="vertical" data-roving-items=".nav-item"><HoverTrack items=".nav-item" />
        {navigation.map(item => {
          const Icon = item.icon;
          const current = section === item.id;
          return <button key={item.id} type="button" className={'nav-item' + (current ? ' selected' : '')} aria-current={current ? 'page' : undefined} aria-label={t(item.label)} title={t(item.label)} onClick={() => onSelect(item.id)}>
            <Icon size={18} weight={current ? 'fill' : 'regular'} aria-hidden="true" /><span className="nav-label">{t(item.label)}</span>
          </button>;
        })}
      </nav>
      <div className="sidebar-footer">
        {!fold.narrow && <button type="button" className="icon-button sidebar-toggle" aria-label={fold.folded ? t('展开侧边栏') : t('收起侧边栏')} title={fold.folded ? t('展开侧边栏') : t('收起侧边栏')} aria-expanded={!fold.folded} onClick={fold.toggle}><SidebarSimple size={18} /></button>}
        {connection}
      </div>
    </aside>
    {phone && <MobileNav items={navigation} tabs={MOBILE_TABS} current={section} onSelect={onSelect} status={connection} open={sheet} onOpen={setSheet} />}
  </>;
}
