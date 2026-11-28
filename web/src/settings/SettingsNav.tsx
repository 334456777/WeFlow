// 设置的分区（#776），列在打开的分区旁边：按内容保存在哪里分组，每个分区用几个字写出它的状态，
// 不打开也能找到分区、看到它的状态。
import type { Icon } from '@phosphor-icons/react';
import { AppWindow, Broom, Gear, HardDrives, Palette, Plugs, Sparkle, TerminalWindow, UserCircle } from '@phosphor-icons/react';
import { HoverTrack } from '../HoverTrack';
import { msg, t } from '../i18n';

/** 设置的一个分区。 */
export type SectionId = 'connection' | 'accounts' | 'appearance' | 'serve' | 'ai' | 'cache' | 'web' | 'console';

interface Section {
  id: SectionId; label: string; icon: Icon;
  /** 读写服务端，所以没有服务端时它自己没有可显示的内容。 */
  online: boolean;
  /** 没有服务端时由示例服务端回答，改动保存在页面里（#896）。 */
  sample?: boolean;
}

/** 按组排列的分区，顺序即显示的顺序；标签显示时经过 t()。 */
export const SECTION_GROUPS: { label: string; sections: Section[] }[] = [
  {
    label: msg('常规'), sections: [
      { id: 'connection', label: msg('连接状态'), icon: Plugs, online: false },
      { id: 'accounts', label: msg('账号'), icon: UserCircle, online: true, sample: true },
      { id: 'appearance', label: msg('外观与浏览'), icon: Palette, online: false },
    ],
  },
  {
    label: 'weflow serve', sections: [
      { id: 'serve', label: msg('启动选项'), icon: HardDrives, online: true, sample: true },
      { id: 'ai', label: msg('AI 见解'), icon: Sparkle, online: true, sample: true },
      { id: 'cache', label: msg('缓存'), icon: Broom, online: true, sample: true },
      { id: 'web', label: msg('网页版本'), icon: AppWindow, online: false },
    ],
  },
  { label: msg('高级'), sections: [{ id: 'console', label: msg('命令控制台'), icon: TerminalWindow, online: true }] },
];

/** 全部分区，按显示的顺序。 */
export const SECTIONS: Section[] = SECTION_GROUPS.flatMap(group => group.sections);

/** 分区在网页语言里的名称。 */
export const sectionLabel = (id: SectionId) => t(SECTIONS.find(section => section.id === id)!.label);

/** 分区列表，每项带着它的状态；没有服务端时，需要服务端的分区标为「连接后可用」。 */
export function SettingsNav({ open, summaries, connected, sample, onOpen }: {
  open: SectionId;
  /** 每个分区用几个字写出的状态。 */
  summaries: Record<SectionId, string>;
  connected: boolean;
  /** 示例服务端正在代替服务端回答（#896）。 */
  sample: boolean;
  onOpen(id: SectionId): void;
}) {
  return <div className="session-pane settings-nav-pane">
    <div className="settings-nav-head"><Gear size={20} aria-hidden="true" /><h1>{t('设置')}</h1></div>
    <nav data-roving="vertical" data-roving-items=".conversation-row" className="conversation-list settings-nav scroll-area" aria-label={t('设置分组')}>
      <HoverTrack items=".conversation-row" />
      {SECTION_GROUPS.map(group => <div key={group.label} className="settings-nav-group" role="group" aria-label={t(group.label)}>
        <p className="settings-nav-caption" aria-hidden="true">{t(group.label)}</p>
        {group.sections.map(({ id, label, icon: Glyph, online, sample: sampled }) => {
          const active = id === open;
          const idle = online && !connected && !(sample && sampled);
          return <button type="button" key={id} className={'conversation-row settings-nav-row' + (active ? ' active' : '') + (idle ? ' idle' : '')}
            aria-current={active ? 'true' : undefined} onClick={() => onOpen(id)}>
            <span className="settings-nav-icon" aria-hidden="true"><Glyph size={18} /></span>
            <span className="conversation-copy">
              <span className="conversation-name">{t(label)}</span>
              <span className="conversation-preview clamp-2">{idle ? t('连接后可用') : summaries[id]}</span>
            </span>
          </button>;
        })}
      </div>)}
    </nav>
  </div>;
}
