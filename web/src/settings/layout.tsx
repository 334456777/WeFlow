// 组成设置页的部件（#776）：带标题的分区、显示为卡片的一组设置行、一行设置（名称和用途，然后是它的控件）、
// 行上的开关、保存多组表单的保存栏、说明操作结果的一行字，以及需要服务端的分区在没有服务端时显示的内容。
import type { ReactNode } from 'react';
import { Plugs } from '@phosphor-icons/react';
import { ApiError } from '../api/client';
import { t } from '../i18n';

/** 服务端说明的出错原因；没有时为 `fallback`。 */
export const reason = (problem: unknown, fallback: string) => (problem instanceof ApiError && problem.message) || fallback;

/**
 * `text` 是否为空，或者是 `min` 到 `max` 之间的整数：这是 `config set` 对相应的键接受的范围
 * （`crates/weflow-core/src/config/keys.rs`）。
 */
export const wholeIn = (text: string, min: number, max: number) => {
  const typed = text.trim();
  return !typed || (/^\d+$/.test(typed) && Number(typed) >= min && Number(typed) <= max);
};

/** 数字输入框里的值不是 `min` 到 `max` 之间的整数时，显示在它下面的警告。 */
export const wholeWarning = (min: number, max: number) => t('要填 {min} 到 {max} 的整数。', { min, max });

/** 操作的结果；错误用 `alert` 角色通知辅助技术。 */
export type Note = { text: string; error?: boolean } | null;
/** 显示结果的一行字；没有结果时不显示。 */
export const NoteLine = ({ note }: { note: Note }) => note && <p className={'account-note' + (note.error ? ' setting-warning' : '')} role={note.error ? 'alert' : undefined} aria-live="polite">{note.text}</p>;

/** 设置的一个分区：标题、说明它包含什么以及保存在哪里的一行字，然后是各组设置。 */
export function SettingsSection({ title, lead, children }: { title: string; lead?: ReactNode; children: ReactNode }) {
  return <section className="settings-section" aria-label={title}>
    <header className="settings-section-head"><h2>{title}</h2>{lead && <p>{lead}</p>}</header>
    {children}
  </section>;
}

/** 同一主题的设置行，放在同一张卡片里；`danger` 的组放不可撤销的操作。 */
export function SettingsGroup({ title, note, actions, danger, children }: {
  title?: string; note?: ReactNode;
  /** 作用于整组的按钮，放在标题旁边。 */
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

/** 一项设置：名称和用途，控件在它们旁边；`wide` 且空间不够时，控件放到它们下面。 */
export function SettingRow({ name, text, wide, children }: { name: ReactNode; text?: ReactNode; wide?: boolean; children?: ReactNode }) {
  return <div className={'setting-row' + (wide ? ' wide' : '')}>
    <div className="setting-copy"><h4>{name}</h4>{text && <p>{text}</p>}</div>
    {children && <div className="setting-control">{children}</div>}
  </div>;
}

/** 开关：画成开关的勾选框，名称为 `label`。 */
export function Toggle({ label, checked, onChange }: { label: string; checked: boolean; onChange(on: boolean): void }) {
  return <label className="toggle"><input type="checkbox" aria-label={label} checked={checked} onChange={event => onChange(event.target.checked)} /><span /></label>;
}

/** 多组表单下方的保存栏，滚动时停在面板底部：显示结果和按钮。 */
export function SaveBar({ note, children }: { note: ReactNode; children: ReactNode }) {
  return <div className="settings-save">
    <div className="settings-save-note">{note}</div>
    <div className="settings-save-actions">{children}</div>
  </div>;
}

/** 需要读取服务端的分区在没有服务端时显示的内容：前往连接状态的按钮。 */
export function NeedsServer({ onConnect }: { onConnect(): void }) {
  return <SettingsGroup>
    <SettingRow name={t('未连接')} text={t('连接 weflow serve 后才能使用。')}>
      <button type="button" className="secondary-button" onClick={onConnect}><Plugs size={14} />{t('查看连接状态')}</button>
    </SettingRow>
  </SettingsGroup>;
}
