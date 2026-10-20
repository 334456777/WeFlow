import { afterEach, describe, expect, it, vi } from 'vitest';
import { previewSize } from './MessageView';
import { canFly, fly, MEDIA_NAME, thumbnailOf } from './previewMotion';

afterEach(() => { document.body.innerHTML = ''; Reflect.deleteProperty(document, 'startViewTransition'); });

// jsdom loads no pictures: one that has, as the browser would say
function loaded(img: HTMLImageElement, width: number, height: number) {
  Object.defineProperty(img, 'complete', { value: true });
  Object.defineProperty(img, 'naturalWidth', { value: width });
  Object.defineProperty(img, 'naturalHeight', { value: height });
  return img;
}

describe('the preview flying from its thumbnail (#194)', () => {
  it('finds the loaded thumbnail of the control it opens from', () => {
    document.body.innerHTML = '<button id="a"><img></button><button id="b"><span><img></span></button><button id="c">no picture</button>';
    const [first, second] = Array.from(document.querySelectorAll('img'));
    // not loaded yet: nothing to fly, nor a shape to size the preview with
    expect(thumbnailOf(document.getElementById('a'))).toBeNull();
    loaded(second, 240, 180);
    expect(thumbnailOf(document.getElementById('c'), second.parentElement, document.getElementById('a'))).toBe(second);
    loaded(first, 120, 160);
    expect(thumbnailOf(document.getElementById('a'), second)).toBe(first);
    expect(thumbnailOf(null, undefined)).toBeNull();
  });

  it('sizes the preview by the thumbnail and shows it until the picture comes', () => {
    const style = previewSize({ src: '/t?x="1"', aspect: 4 / 3 });
    expect(style.aspectRatio).toBe(String(4 / 3));
    expect(style.width).toBe('min(960px, 100vw - 32px, (100dvh - 120px) * 1.3333)');
    expect(style.background).toBe('url("/t?x=\\"1\\"") center / contain no-repeat');
  });

  it('flies only where the browser can and no less motion is asked for', () => {
    expect(canFly(false)).toBe(false);
    Object.defineProperty(document, 'startViewTransition', { configurable: true, value: () => ({}) });
    expect(canFly(false)).toBe(true);
    expect(canFly(true)).toBe(false);
  });

  it('names the thumbnail before the change and what it turned into after, then neither', async () => {
    document.body.innerHTML = '<img id="from"><img id="to" hidden>';
    const from = document.getElementById('from')!;
    const to = document.getElementById('to')!;
    const seen: string[] = [];
    let done!: () => void;
    Object.defineProperty(document, 'startViewTransition', {
      configurable: true,
      value: (update: () => void) => {
        seen.push('before: ' + from.style.viewTransitionName);
        update();
        seen.push('after: ' + from.style.viewTransitionName + ' / ' + to.style.viewTransitionName);
        return { finished: new Promise<void>(resolve => { done = resolve; }) };
      },
    });
    const change = vi.fn(() => { to.hidden = false; });
    const flight = fly(from, change, () => (to.hidden ? null : to));
    expect(change).toHaveBeenCalledTimes(1);
    expect(seen).toEqual(['before: ' + MEDIA_NAME, 'after:  / ' + MEDIA_NAME]);
    done();
    await flight;
    expect(to.style.viewTransitionName).toBe('');
    expect(from.style.viewTransitionName).toBe('');
  });
});
