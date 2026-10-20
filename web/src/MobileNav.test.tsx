import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AddressBook, ChatCircleDots, Gear, Wallet } from '@phosphor-icons/react';
import { MobileBar, MobileNav } from './MobileNav';

afterEach(cleanup);

const items = [
  { id: 'chat', label: '聊天', icon: ChatCircleDots },
  { id: 'contacts', label: '通讯录', icon: AddressBook },
  { id: 'payments', label: '微信支付', icon: Wallet },
  { id: 'settings', label: '设置', icon: Gear },
] as const;
type Id = typeof items[number]['id'];

function Nav({ onSelect }: { onSelect(id: Id): void }) {
  const [current, setCurrent] = useState<Id>('chat');
  const [open, setOpen] = useState(false);
  return <MobileNav items={items} tabs={['chat', 'contacts']} current={current} status={<span>实时</span>} open={open} onOpen={setOpen}
    onSelect={id => { setCurrent(id); onSelect(id); }} />;
}

describe('the navigation on a phone (#335)', () => {
  it('shows the tabs, the current one marked, and 「更多」 for the rest', () => {
    render(<Nav onSelect={vi.fn()} />);
    const tabs = screen.getAllByRole('button');
    expect(tabs.map(t => t.textContent)).toEqual(['聊天', '通讯录', '更多']);
    expect(tabs[0].getAttribute('aria-current')).toBe('page');
    expect(tabs[2].getAttribute('aria-current')).toBeNull();
  });

  it('opens the other pages from 「更多」, and names the one chosen there on the tab', () => {
    // jsdom has no modal dialogs
    HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) { this.open = true; };
    HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) { this.open = false; };
    const onSelect = vi.fn();
    render(<Nav onSelect={onSelect} />);
    fireEvent.click(screen.getByRole('button', { name: '更多' }));
    const sheet = screen.getByRole('dialog', { name: '更多页面', hidden: true });
    expect([...sheet.querySelectorAll('.nav-sheet-item')].map(b => b.textContent)).toEqual(['微信支付', '设置']);
    expect(sheet.textContent).toContain('实时');
    fireEvent.click(screen.getByRole('button', { name: '设置', hidden: true }));
    expect(onSelect).toHaveBeenCalledWith('settings');
    const more = screen.getAllByRole('button', { hidden: true }).at(2)!;
    expect(more.textContent).toBe('设置');
    expect(more.getAttribute('aria-current')).toBe('page');
  });

  it('puts the way back over a detail', () => {
    const onBack = vi.fn();
    render(<MobileBar label="返回对话列表" title="林沐" onBack={onBack} />);
    fireEvent.click(screen.getByRole('button', { name: '返回对话列表' }));
    expect(onBack).toHaveBeenCalledTimes(1);
    expect(screen.getByText('林沐')).toBeTruthy();
  });
});
