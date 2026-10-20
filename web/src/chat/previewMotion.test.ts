import { afterEach, describe, expect, it, vi } from 'vitest';
import { previewSize } from './MessageView';
import { canFly, fly, frameVars, MEDIA_NAME, thumbnailOf } from './previewMotion';

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
    expect(style.width).toBe('min(960px, 100vw - var(--preview-room-x), (100dvh - var(--preview-room-y)) * 1.3333)');
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

describe('the frame flying with the picture (#339)', () => {
  it('scales and shifts the frame so that its picture lies on the thumbnail', () => {
    const frame = { left: 100, top: 80, width: 440, height: 420 };
    const media = { left: 120, top: 130, width: 400, height: 300 };
    const thumb = { left: 10, top: 500, width: 100, height: 75 };
    const vars = frameVars(frame, media, thumb);
    expect(vars).toMatchObject({ '--frame-w': '440px', '--frame-h': '420px', '--frame-sx': '0.25', '--frame-sy': '0.25', '--frame-tx': '-5px', '--frame-ty': '-12.5px' });
    // at the start the frame's box is the thumbnail's, and its picture, scaled, starts at the box's corner
    const at = (axis: 'x' | 'y') => {
      const [corner, offset, scale] = axis === 'x' ? [thumb.left, media.left - frame.left, 0.25] : [thumb.top, media.top - frame.top, 0.25];
      return corner + parseFloat(vars['--frame-t' + axis]) + offset * scale;
    };
    expect([at('x'), at('y')]).toEqual([thumb.left, thumb.top]);
    expect(media.width * 0.25).toBe(thumb.width);
  });
});
