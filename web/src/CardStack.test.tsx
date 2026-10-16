import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { CardStack, loadLayout, type Card } from './CardStack';

const KEY = 'test.cards';
const cards: Card[] = [
  { id: 'a', title: '甲', summary: '一', content: <p>甲的内容</p> },
  { id: 'b', title: '乙', summary: '二', content: <p>乙的内容</p> },
  { id: 'c', title: '丙', content: <p>丙的内容</p> },
];

afterEach(() => {
  cleanup();
  localStorage.clear();
});

const titles = () => screen.getAllByRole('listitem').map(item => item.querySelector('.card-title')?.textContent);

describe('CardStack', () => {
  it('opens and closes a card from its header and remembers it', () => {
    render(<CardStack cards={cards} storageKey={KEY} defaultClosed={['b']} label="卡片" />);
    const header = screen.getByRole('button', { name: /乙/ });
    expect(header.getAttribute('aria-expanded')).toBe('false');
    expect(document.getElementById('card-body-b')?.hasAttribute('inert')).toBe(true);

    fireEvent.click(header);
    expect(header.getAttribute('aria-expanded')).toBe('true');
    expect(document.getElementById('card-body-b')?.hasAttribute('inert')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: /甲/ }));
    expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual({ order: ['a', 'b', 'c'], closed: ['a'] });
  });

  it('moves a card with Alt+arrows, keeps the focus on it and restores the order', () => {
    render(<CardStack cards={cards} storageKey={KEY} defaultClosed={[]} label="卡片" />);
    const header = screen.getByRole('button', { name: /甲/ });
    header.focus();
    fireEvent.keyDown(header, { key: 'ArrowDown', altKey: true });
    fireEvent.keyDown(header, { key: 'ArrowDown', altKey: true });
    expect(titles()).toEqual(['乙', '丙', '甲']);
    expect(document.activeElement?.id).toBe('card-a');

    fireEvent.keyDown(header, { key: 'ArrowDown', altKey: true });
    fireEvent.keyDown(header, { key: 'ArrowDown' });
    expect(titles()).toEqual(['乙', '丙', '甲']);

    cleanup();
    render(<CardStack cards={cards} storageKey={KEY} defaultClosed={[]} label="卡片" />);
    expect(titles()).toEqual(['乙', '丙', '甲']);
  });
});

describe('hidden cards', () => {
  it('are left out, keep their place while the others move, and come back there', () => {
    const { rerender } = render(<CardStack cards={cards} hidden={['b']} storageKey={KEY} defaultClosed={[]} label="卡片" />);
    expect(titles()).toEqual(['甲', '丙']);
    const header = screen.getByRole('button', { name: /丙/ });
    fireEvent.keyDown(header, { key: 'ArrowUp', altKey: true });
    expect(titles()).toEqual(['丙', '甲']);
    expect(JSON.parse(localStorage.getItem(KEY)!).order).toEqual(['c', 'b', 'a']);
    rerender(<CardStack cards={cards} hidden={[]} storageKey={KEY} defaultClosed={[]} label="卡片" />);
    expect(titles()).toEqual(['丙', '乙', '甲']);
  });
});

describe('loadLayout', () => {
  it('drops unknown cards, appends new ones and ignores a broken entry', () => {
    localStorage.setItem(KEY, JSON.stringify({ order: ['c', 'gone', 'a'], closed: ['gone', 'c'] }));
    expect(loadLayout(KEY, ['a', 'b', 'c'], [])).toEqual({ order: ['c', 'a', 'b'], closed: ['c'] });
    localStorage.setItem(KEY, '{broken');
    expect(loadLayout(KEY, ['a', 'b'], ['b'])).toEqual({ order: ['a', 'b'], closed: ['b'] });
  });
});
