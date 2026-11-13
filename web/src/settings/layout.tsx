// What the settings are built of (#776): a section under its heading, groups of rows as raised cards, a row of a
// setting (its name and what it does, then its control), the switch of a row, the bar that saves a form of several
// groups, and what a section that needs the server shows without it.
import type { ReactNode } from 'react';
import { Plugs } from '@phosphor-icons/react';
import { t } from '../i18n';

/** A section of the settings: its heading, a line on what it holds and where that is kept, then its groups. */
export function SettingsSection({ title, lead, children }: { title: string; lead?: ReactNode; children: ReactNode }) {
  return <section className="settings-section" aria-label={title}>
    <header className="settings-section-head"><h2>{title}</h2>{lead && <p>{lead}</p>}</header>
    {children}
  </section>;
}

/** Rows on one subject, raised together; a danger group holds what cannot be undone. */
export function SettingsGroup({ title, note, actions, danger, children }: {
  title?: string; note?: ReactNode;
  /** Buttons that act on the whole group, beside its title. */
  actions?: ReactNode;
  danger?: boolean;
  children: ReactNode;
}) {
  return <div className={'settings-group' + (danger ? ' danger' : '')}>
    {(title || actions) && <header className="settings-group-head">
      <div>{title && <h3>{title}</h3>}{note && <p>{note}</p>}</div>
      {actions && <div className="settings-group-actions">{actions}</div>}
    </header>}
    {children}
  </div>;
}

/** A setting: its name and what it does, its control beside them, or under them when `wide` and the room is short. */
export function SettingRow({ name, text, wide, children }: { name: ReactNode; text?: ReactNode; wide?: boolean; children?: ReactNode }) {
  return <div className={'setting-row' + (wide ? ' wide' : '')}>
    <div className="setting-copy"><h4>{name}</h4>{text && <p>{text}</p>}</div>
    {children && <div className="setting-control">{children}</div>}
  </div>;
}

export function Toggle({ label, checked, onChange }: { label: string; checked: boolean; onChange(on: boolean): void }) {
  return <label className="toggle"><input type="checkbox" aria-label={label} checked={checked} onChange={event => onChange(event.target.checked)} /><span /></label>;
}

/** Under a form of several groups, held at the bottom of the pane while it scrolls: what happened, and its buttons. */
export function SaveBar({ note, children }: { note: ReactNode; children: ReactNode }) {
  return <div className="settings-save">
    <div className="settings-save-note">{note}</div>
    <div className="settings-save-actions">{children}</div>
  </div>;
}

/** A section that reads the server, while there is none: the way to the connection. */
export function NeedsServer({ onConnect }: { onConnect(): void }) {
  return <SettingsGroup>
    <SettingRow name={t('未连接')} text={t('连接 weflow serve 后才能使用。')}>
      <button type="button" className="secondary-button" onClick={onConnect}><Plugs size={14} />{t('查看连接状态')}</button>
    </SettingRow>
  </SettingsGroup>;
}
