import { cleanup, fireEvent, render } from '@testing-library/react';
import { useRef } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { Scrollbar } from './Scrollbar';

afterEach(cleanup);

function Area() {
  const ref = useRef<HTMLDivElement>(null);
  return <><div ref={ref} className="scroll-area" data-testid="area"><p>content</p></div><Scrollbar target={ref} /></>;
}

// jsdom has no layout: give the area a size before the bar measures it.
function sized(height: number, content: number) {
  const proto = HTMLElement.prototype;
  const saved = [Object.getOwnPropertyDescriptor(proto, 'clientHeight'), Object.getOwnPropertyDescriptor(proto, 'scrollHeight')];
  Object.defineProperty(proto, 'clientHeight', { configurable: true, get() { return this.classList?.contains('scroll-area') ? height : 0; } });
  Object.defineProperty(proto, 'scrollHeight', { configurable: true, get() { return this.classList?.contains('scroll-area') ? content : 0; } });
  return () => {
    for (const [name, descriptor] of [['clientHeight', saved[0]], ['scrollHeight', saved[1]]] as const) {
      if (descriptor) Object.defineProperty(proto, name, descriptor);
      else Reflect.deleteProperty(proto, name);
    }
  };
}

describe('Scrollbar', () => {
  it('stays away while nothing overflows', () => {
    const restore = sized(400, 400);
    try {
      const { container } = render(<Area />);
      expect(container.querySelector('.scrollbar')!.classList.contains('scrollable')).toBe(false);
    } finally { restore(); }
  });

  it('sizes and places the thumb, shows while scrolling, and scrolls when the thumb is dragged', () => {
    const restore = sized(408, 4080);
    try {
      const { container, getByTestId } = render(<Area />);
      const bar = container.querySelector('.scrollbar') as HTMLElement;
      const thumb = container.querySelector('.scrollbar-thumb') as HTMLElement;
      const area = getByTestId('area');
      expect(bar.classList.contains('scrollable')).toBe(true);
      // The track is the area less 4px at each end: 400px; the area shows a tenth of the content, so does the thumb.
      expect(bar.style.height).toBe('400px');
      expect(thumb.style.height).toBe('40px');

      area.scrollTop = 1836;
      fireEvent.scroll(area);
      expect(bar.classList.contains('active')).toBe(true);
      expect(thumb.style.transform).toBe('translateY(180px)');

      // 36px is a tenth of the 360px the thumb can travel, so a tenth of the 3672px the area can scroll.
      fireEvent.pointerDown(thumb, { button: 0, clientY: 200, pointerId: 1 });
      expect(bar.classList.contains('dragging')).toBe(true);
      fireEvent.pointerMove(thumb, { clientY: 236, pointerId: 1 });
      expect(area.scrollTop).toBeCloseTo(1836 + 367.2);
      fireEvent.pointerUp(thumb, { pointerId: 1 });
      expect(bar.classList.contains('dragging')).toBe(false);
    } finally { restore(); }
  });
});
