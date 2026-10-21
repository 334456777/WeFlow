import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from '#test';
import { step, useRovingFocus, useSearchKey } from './roving';

afterEach(cleanup);

function Page({ chosen = 1, keys }: { chosen?: number; keys?: 'off' }) {
  useRovingFocus();
  useSearchKey();
  return <main>
    <input type="search" aria-label="搜索" />
    <nav data-roving="vertical" data-roving-items=".item" data-roving-keys={keys}>
      {['a', 'b', 'c', 'd'].map((n, i) => <button key={n} type="button" className={'item' + (i === chosen ? ' selected' : '')} disabled={n === 'd'}>{n}</button>)}
    </nav>
    <div className="choice-chips" role="group" aria-label="格式">
      <button type="button" aria-pressed="false">HTML</button><button type="button" aria-pressed="true">JSON</button>
    </div>
  </main>;
}
const tabStops = (container: HTMLElement, selector: string) => Array.from(container.querySelectorAll<HTMLElement>(selector), b => b.tabIndex);

describe('roving focus', () => {
  it('moves by the keys of each orientation, and stays within the group', () => {
    expect(step('ArrowDown', 0, 5, 'vertical')).toBe(1);
    expect(step('ArrowRight', 0, 5, 'vertical')).toBeNull();
    expect(step('ArrowRight', 4, 5, 'horizontal')).toBe(4);
    expect(step('ArrowDown', 1, 9, 'grid', 3)).toBe(4);
    expect(step('ArrowLeft', 1, 9, 'grid', 3)).toBe(0);
    expect([step('Home', 3, 30, 'vertical'), step('End', 3, 30, 'vertical'), step('PageDown', 3, 30, 'vertical'), step('PageUp', 3, 30, 'vertical')]).toEqual([0, 29, 13, 0]);
    expect(step('PageDown', 0, 5, 'horizontal')).toBeNull();
  });

  it('leaves one Tab stop per group, the chosen item, and moves the focus and the stop with the arrows', () => {
    const { container } = render(<Page />);
    expect(tabStops(container, 'nav button')).toEqual([-1, 0, -1, 0]);
    expect(tabStops(container, '.choice-chips button')).toEqual([-1, 0]);
    const items = container.querySelectorAll<HTMLButtonElement>('nav button');
    items[1].focus();
    fireEvent.keyDown(items[1], { key: 'ArrowDown' });
    expect(document.activeElement).toBe(items[2]);
    // the disabled one is skipped: the end is the last that can take the focus
    fireEvent.keyDown(items[2], { key: 'End' });
    expect(document.activeElement).toBe(items[2]);
    fireEvent.keyDown(items[2], { key: 'Home' });
    expect(document.activeElement).toBe(items[0]);
    expect(tabStops(container, 'nav button').slice(0, 3)).toEqual([0, -1, -1]);
    const chips = container.querySelectorAll<HTMLButtonElement>('.choice-chips button');
    chips[1].focus();
    fireEvent.keyDown(chips[1], { key: 'ArrowLeft' });
    expect(document.activeElement).toBe(chips[0]);
  });

  it('keeps the stop but leaves the keys to a group that takes them itself', () => {
    const { container } = render(<Page keys="off" />);
    const items = container.querySelectorAll<HTMLButtonElement>('nav button');
    items[1].focus();
    fireEvent.keyDown(items[1], { key: 'ArrowDown' });
    expect(document.activeElement).toBe(items[1]);
    expect(tabStops(container, 'nav button').slice(0, 3)).toEqual([-1, 0, -1]);
  });

  it('follows the choice when it changes, and puts / into the search field but not while typing', async () => {
    const { container, rerender } = render(<Page chosen={0} />);
    rerender(<Page chosen={2} />);
    await act(async () => { await new Promise(r => requestAnimationFrame(() => r(null))); });
    expect(tabStops(container, 'nav button').slice(0, 3)).toEqual([-1, -1, 0]);
    const search = container.querySelector('input')!;
    // happy-dom lays nothing out: offsetParent is null, so count the field as showing
    Object.defineProperty(search, 'offsetParent', { configurable: true, get: () => document.body });
    fireEvent.keyDown(document.body, { key: '/' });
    expect(document.activeElement).toBe(search);
    const typed = fireEvent.keyDown(search, { key: '/' });
    expect(typed).toBe(true);
  });
});
