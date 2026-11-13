// The sections of the settings (#776), listed beside the one open: grouped by where what they hold is kept, each
// with its state in a few words, so a section can be found, and its state read, without opening it.
import type { Icon } from '@phosphor-icons/react';
import { FileHtml, FolderOpen, Gear, HardDrives, Palette, Plugs, Sparkle, Terminal, UserCircle } from '@phosphor-icons/react';
import { HoverTrack } from '../HoverTrack';
import { msg, t } from '../i18n';

export type SectionId = 'connection' | 'accounts' | 'appearance' | 'serve' | 'ai' | 'cache' | 'web' | 'console';

interface Section {
  id: SectionId; label: string; icon: Icon;
  /** It reads or writes the server, so it shows nothing of its own without one. */
  online: boolean;
}

/** The sections by group, in their order; labels shown through t(). */
export const SECTION_GROUPS: { label: string; sections: Section[] }[] = [
  {
    label: msg('常规'), sections: [
      { id: 'connection', label: msg('连接状态'), icon: Plugs, online: false },
      { id: 'accounts', label: msg('账号'), icon: UserCircle, online: true },
      { id: 'appearance', label: msg('外观与浏览'), icon: Palette, online: false },
    ],
  },
  {
    label: 'weflow serve', sections: [
      { id: 'serve', label: msg('启动选项'), icon: HardDrives, online: true },
      { id: 'ai', label: msg('AI 见解'), icon: Sparkle, online: true },
      { id: 'cache', label: msg('缓存'), icon: FolderOpen, online: true },
      { id: 'web', label: msg('网页版本'), icon: FileHtml, online: false },
    ],
  },
  { label: msg('高级'), sections: [{ id: 'console', label: msg('命令控制台'), icon: Terminal, online: true }] },
];

export const SECTIONS: Section[] = SECTION_GROUPS.flatMap(group => group.sections);

export const sectionLabel = (id: SectionId) => t(SECTIONS.find(section => section.id === id)!.label);

export function SettingsNav({ open, summaries, connected, onOpen }: {
  open: SectionId;
  /** Each section's state in a few words. */
  summaries: Record<SectionId, string>;
  connected: boolean;
  onOpen(id: SectionId): void;
}) {
  return <div className="session-pane settings-nav-pane">
    <div className="settings-nav-head"><Gear size={20} aria-hidden="true" /><h1>{t('设置')}</h1></div>
    <nav data-roving="vertical" data-roving-items=".conversation-row" className="conversation-list settings-nav scroll-area" aria-label={t('设置分组')}>
      <HoverTrack items=".conversation-row" />
      {SECTION_GROUPS.map(group => <div key={group.label} className="settings-nav-group" role="group" aria-label={t(group.label)}>
        <p className="settings-nav-caption" aria-hidden="true">{t(group.label)}</p>
        {group.sections.map(({ id, label, icon: Glyph, online }) => {
          const active = id === open;
          const idle = online && !connected;
          return <button type="button" key={id} className={'conversation-row settings-nav-row' + (active ? ' active' : '') + (idle ? ' idle' : '')}
            aria-current={active ? 'true' : undefined} onClick={() => onOpen(id)}>
            <span className="settings-nav-icon" aria-hidden="true"><Glyph size={18} /></span>
            <span className="conversation-copy">
              <span className="conversation-name">{t(label)}</span>
              <span className="conversation-preview clamp-1">{idle ? t('连接后可用') : summaries[id]}</span>
            </span>
          </button>;
        })}
      </div>)}
    </nav>
  </div>;
}
