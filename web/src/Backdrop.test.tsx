import { act, cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from '#test';
import { Backdrop, backdropCount } from './Backdrop';
import { turnPage } from './pageTurn';
import { applySkin, SKINS } from './skin';

afterEach(() => {
  cleanup();
  document.documentElement.style.removeProperty('--backdrop-count');
});

describe('Backdrop (#832)', () => {
  it('is hidden from assistive technology, and takes turns among the skin’s decorations, drawn anew at each', () => {
    document.documentElement.style.setProperty('--backdrop-count', '3');
    const { container } = render(<Backdrop />);
    const layer = () => container.querySelector<HTMLElement>('.backdrop')!;
    expect(layer().getAttribute('aria-hidden')).toBe('true');
    const first = layer();
    const turnOf = () => Number(layer().dataset.turn);
    const before = turnOf();
    act(() => turnPage());
    expect(layer()).not.toBe(first);
    expect(turnOf()).toBe((before + 1) % 3);
    act(() => { turnPage(); turnPage(); });
    expect(turnOf()).toBe(before);

    // another item's detail turns the decoration, but not the page
    const pageOf = () => Number(layer().dataset.page);
    const page = pageOf();
    const shown = layer();
    act(() => turnPage({ detail: true }));
    expect(layer()).not.toBe(shown);
    expect(turnOf()).toBe((before + 1) % 3);
    expect(pageOf()).toBe(page);
    act(() => turnPage());
    expect(pageOf()).toBe((page + 1) % 3);
  });

  it('is not there for a skin without decorations, and comes with a skin that has them', async () => {
    const { container } = render(<Backdrop />);
    expect(container.querySelector('.backdrop')).toBeNull();
    // a skin whose sheet (here, the root's own style) has two
    const decorated = { id: 'decorated', label: 'decorated', themes: ['light'] as const };
    SKINS.push(decorated);
    try {
      document.documentElement.style.setProperty('--backdrop-count', '2');
      await act(async () => { await applySkin(decorated); });
      expect(container.querySelector('.backdrop')).not.toBeNull();
    } finally {
      SKINS.splice(SKINS.indexOf(decorated), 1);
      await act(async () => { await applySkin(SKINS[0]); });
    }
  });

  it('counts no decoration for a skin without one, or a count that is no number', () => {
    expect(backdropCount()).toBe(0);
    document.documentElement.style.setProperty('--backdrop-count', 'some');
    expect(backdropCount()).toBe(0);
    document.documentElement.style.setProperty('--backdrop-count', '4');
    expect(backdropCount()).toBe(4);
  });
});
