import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from '#test';
import type { ParsedMessage } from '../api/messages';
import { withGallery } from './MessageView';
import { FIT, keep, zoomAt, ZoomImage } from './ZoomImage';

afterEach(cleanup);

const message = (n: number, localType: number): ParsedMessage =>
  ({ _cursor: 'c' + n, localId: n, localType, createTime: 1_700_000_000 + n, isSend: 0, senderUsername: 'wxid_a', parsedContent: '' }) as ParsedMessage;

// happy-dom has no layout: the picture is 400 × 300 at 1×
function sized() {
  const proto = HTMLElement.prototype;
  const saved = [Object.getOwnPropertyDescriptor(proto, 'offsetWidth'), Object.getOwnPropertyDescriptor(proto, 'offsetHeight')];
  Object.defineProperty(proto, 'offsetWidth', { configurable: true, get() { return this.tagName === 'IMG' ? 400 : 0; } });
  Object.defineProperty(proto, 'offsetHeight', { configurable: true, get() { return this.tagName === 'IMG' ? 300 : 0; } });
  return () => {
    for (const [name, d] of [['offsetWidth', saved[0]], ['offsetHeight', saved[1]]] as const) {
      if (d) Object.defineProperty(proto, name, d); else Reflect.deleteProperty(proto, name);
    }
  };
}

describe('zooming a picture', () => {
  it('keeps the point under the pointer where it is, and the picture inside the frame', () => {
    const v = zoomAt(FIT, 2, 100, 50, 400, 300);
    // the point 100px right of the centre was 100px into the picture; at 2× that is 200px, so the picture moves -100
    expect(v).toEqual({ scale: 2, x: -100, y: -50 });
    expect(zoomAt(v, 4, 100, 50, 400, 300)).toEqual({ scale: 4, x: -300, y: -150 });
    // at 2× it grew 200 × 150 each way, so it moves at most half of that
    expect(keep({ scale: 2, x: 500, y: -500 }, 400, 300)).toEqual({ scale: 2, x: 200, y: -150 });
    expect(zoomAt(v, 0.5, 0, 0, 400, 300)).toEqual(FIT);
    expect(zoomAt(FIT, 100, 0, 0, 400, 300).scale).toBe(8);
  });

  it('zooms with the wheel, pans only past 4px, and goes back to fit on a double click', () => {
    const restore = sized();
    try {
      const { container } = render(<ZoomImage src="/a.jpg" alt="图片" onError={() => {}} />);
      const frame = container.querySelector('.zoom-frame') as HTMLElement;
      const img = container.querySelector('img')!;
      fireEvent.wheel(frame, { deltaY: -500, clientX: 0, clientY: 0 });
      const scale = Number(/scale\(([\d.]+)\)/.exec(img.style.transform)![1]);
      expect(scale).toBeCloseTo(Math.exp(0.75), 5);
      expect(frame.classList.contains('zoomed')).toBe(true);
      const before = img.style.transform;
      fireEvent.pointerDown(frame, { button: 0, clientX: 100, clientY: 100, pointerId: 1 });
      fireEvent.pointerMove(frame, { clientX: 102, clientY: 102, pointerId: 1 });
      expect(img.style.transform).toBe(before);
      fireEvent.pointerMove(frame, { clientX: 130, clientY: 110, pointerId: 1 });
      expect(img.style.transform).not.toBe(before);
      fireEvent.pointerUp(frame, { pointerId: 1 });
      fireEvent.doubleClick(frame, { clientX: 0, clientY: 0 });
      expect(img.style.transform).toBe('translate(0px, 0px) scale(1)');
    } finally { restore(); }
  });
});

describe('the gallery of a conversation', () => {
  it('goes through the pictures and videos read so far, from the one opened', () => {
    const messages = [message(1, 3), message(2, 1), message(3, 43), message(4, 3)];
    const preview = withGallery('wxid_a', messages, messages[2])!;
    expect(preview.type).toBe('video');
    if (preview.type === 'file') throw new Error();
    expect(preview.gallery?.index).toBe(1);
    expect(preview.gallery?.items.map(p => p.type)).toEqual(['image', 'video', 'image']);
    // a lone picture has nothing to go through, and text has no preview
    expect(withGallery('wxid_a', [message(1, 3)], message(1, 3))).not.toHaveProperty('gallery');
    expect(withGallery('wxid_a', messages, messages[1])).toBeNull();
  });
});
