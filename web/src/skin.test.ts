import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from '#test';
import { applySkin, DEFAULT_SKIN, savedSkin, SKIN_KEY, SKINS, type Skin, useSkin } from './skin';

const root = document.documentElement;

/** Another skin, whose sheet arrives or fails when told to; each test names its own, as a sheet is fetched once. */
function another(id: string, shapes?: boolean) {
  let loads = 0;
  let settle: { resolve(): void; reject(): void } | null = null;
  const skin: Skin = {
    id, label: id, themes: ['light'], shapes,
    sheet: () => {
      loads += 1;
      return new Promise((resolve, reject) => { settle = { resolve: () => resolve({ default: `:root[data-skin='${id}'] { --x: 1; }` }), reject }; });
    },
  };
  SKINS.push(skin);
  return {
    skin,
    loads: () => loads,
    arrive: async () => { await act(async () => settle!.resolve()); },
    fail: async () => { await act(async () => settle!.reject()); },
  };
}

beforeEach(() => {
  localStorage.clear(); delete root.dataset.skin; delete root.dataset.skinPending; delete root.dataset.shapes;
  document.head.querySelectorAll('style[data-skin]').forEach(sheet => sheet.remove());
});
afterEach(() => { cleanup(); SKINS.splice(1); });

describe('the skin (#830)', () => {
  it('is the default without a choice, and for a skin this page does not have', () => {
    expect(savedSkin().id).toBe(DEFAULT_SKIN);
    localStorage.setItem(SKIN_KEY, 'gone');
    expect(savedSkin().id).toBe(DEFAULT_SKIN);
  });

  it('goes on once its sheet is here, fetched once, and shows the page the first paint hid', async () => {
    const paper = another('paper');
    localStorage.setItem(SKIN_KEY, 'paper');
    root.dataset.skinPending = '';
    const ready = applySkin(savedSkin());
    expect(root.dataset.skin).toBeUndefined();
    expect(root.dataset.skinPending).toBe('');
    await paper.arrive();
    expect((await ready).id).toBe('paper');
    expect(root.dataset.skin).toBe('paper');
    expect(root.dataset.skinPending).toBeUndefined();
    expect(document.head.querySelector('style[data-skin="paper"]')?.textContent).toBe(":root[data-skin='paper'] { --x: 1; }");
    await applySkin(paper.skin);
    expect(paper.loads()).toBe(1);
    expect(document.head.querySelectorAll('style[data-skin="paper"]')).toHaveLength(1);
  });

  it('leaves the default on when its sheet cannot be fetched, and tries again the next time', async () => {
    const paper = another('torn');
    root.dataset.skinPending = '';
    const ready = applySkin(paper.skin);
    await paper.fail();
    expect((await ready).id).toBe(DEFAULT_SKIN);
    expect(root.dataset.skin).toBe(DEFAULT_SKIN);
    expect(root.dataset.skinPending).toBeUndefined();
    void applySkin(paper.skin);
    expect(paper.loads()).toBe(2);
  });

  it('is kept when chosen, and forgotten on going back to the default', async () => {
    const paper = another('kept');
    const { result } = renderHook(() => useSkin());
    expect(result.current[0].id).toBe(DEFAULT_SKIN);
    act(() => result.current[1]('kept'));
    expect(result.current[0].id).toBe('kept');
    expect(localStorage.getItem(SKIN_KEY)).toBe('kept');
    await paper.arrive();
    expect(root.dataset.skin).toBe('kept');
    await act(async () => result.current[1](DEFAULT_SKIN));
    expect(localStorage.getItem(SKIN_KEY)).toBeNull();
    expect(root.dataset.skin).toBe(DEFAULT_SKIN);
  });

  it('turns on the shape layers with a skin that has them, and off with one that has none (#831)', async () => {
    const stickers = another('stickers', true);
    const ready = applySkin(stickers.skin);
    expect(root.dataset.shapes).toBeUndefined();
    await stickers.arrive();
    await ready;
    expect(root.dataset.shapes).toBe('');
    await applySkin(SKINS[0]);
    expect(root.dataset.shapes).toBeUndefined();
  });

  it('takes the last choice when an earlier one’s sheet comes after it', async () => {
    const slow = another('slow');
    const quick = another('quick');
    const { result } = renderHook(() => useSkin());
    act(() => result.current[1]('slow'));
    act(() => result.current[1]('quick'));
    await quick.arrive();
    await slow.arrive();
    expect(root.dataset.skin).toBe('quick');
    expect(result.current[0].id).toBe('quick');
  });

  it('shows the default as chosen when the sheet of the one chosen cannot be fetched', async () => {
    const paper = another('lost');
    const { result } = renderHook(() => useSkin());
    act(() => result.current[1]('lost'));
    await paper.fail();
    expect(result.current[0].id).toBe(DEFAULT_SKIN);
    expect(root.dataset.skin).toBe(DEFAULT_SKIN);
  });
});
