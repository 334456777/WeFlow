import { cleanup, fireEvent, render } from '@testing-library/react';
import { useRef } from 'react';
import { afterEach, describe, expect, it, vi } from '#test';
import { SCROLL_INTENT, Scrollbar } from './Scrollbar';

afterEach(cleanup);

function Area() {
  const ref = useRef<HTMLDivElement>(null);
  return <><div ref={ref} className="scroll-area" data-testid="area"><p>content</p></div><Scrollbar target={ref} /></>;
}

/** One bar for two pages, as a panel that shows one page and then another keeps its bar (#836). */
function Pages({ page }: { page: 'a' | 'b' }) {
  const a = useRef<HTMLDivElement>(null);
  const b = useRef<HTMLDivElement>(null);
  return <>
    {page === 'a'
      ? <div ref={a} className="scroll-area" data-testid="a"><p>a</p></div>
      : <div ref={b} className="scroll-area" data-testid="b"><p>b</p></div>}
    <Scrollbar target={page === 'a' ? a : b} />
  </>;
}

// happy-dom has no layout: give the area a size before the bar measures it.
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

  it('fades the edges there is more beyond, and neither when nothing overflows (#831)', () => {
    let restore = sized(400, 1200);
    try {
      const { container, getByTestId, unmount } = render(<Area />);
      const area = getByTestId('area');
      const frame = container.querySelector<HTMLElement>('.scroll-fades')!;
      const shown = () => ['top', 'bottom'].filter(side => frame.querySelector('.scroll-fade.' + side)!.classList.contains('shown'));
      // over the area, as tall as it (the test's areas have no width)
      expect(frame.classList.contains('scrollable')).toBe(true);
      expect(frame.style.height).toBe('400px');
      expect(shown()).toEqual(['bottom']);
      area.scrollTop = 300;
      fireEvent.scroll(area);
      expect(shown()).toEqual(['top', 'bottom']);
      area.scrollTop = 800;
      fireEvent.scroll(area);
      expect(shown()).toEqual(['top']);
      unmount();
      restore();
      restore = sized(400, 400);
      const again = render(<Area />).container;
      expect(again.querySelector('.scroll-fades')!.classList.contains('scrollable')).toBe(false);
      expect(again.querySelectorAll('.scroll-fade.shown')).toHaveLength(0);
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

      // 36px is a tenth of the 360px the thumb can travel, so a tenth of the 3672px the area can scroll; the area
      // hears that a person scrolls it (the thumb is not inside it)
      const intent = vi.fn();
      area.addEventListener(SCROLL_INTENT, intent);
      fireEvent.pointerDown(thumb, { button: 0, clientY: 200, pointerId: 1 });
      expect(bar.classList.contains('dragging')).toBe(true);
      expect(intent).toHaveBeenCalledTimes(1);
      fireEvent.pointerMove(thumb, { clientY: 236, pointerId: 1 });
      expect(area.scrollTop).toBeCloseTo(1836 + 367.2);
      // an area that scrolls smoothly is moved at once, or it would trail behind the pointer (#159)
      const scrollTo = vi.fn();
      area.scrollTo = scrollTo as typeof area.scrollTo;
      fireEvent.pointerMove(thumb, { clientY: 272, pointerId: 1 });
      expect(scrollTo).toHaveBeenLastCalledWith({ top: expect.closeTo(1836 + 734.4), behavior: 'instant' });
      fireEvent.pointerUp(thumb, { pointerId: 1 });
      expect(bar.classList.contains('dragging')).toBe(false);
    } finally { restore(); }
  });

  it('shows nothing of the page before once another takes its place', () => {
    const restore = sized(400, 1200);
    try {
      const { container, getByTestId, rerender } = render(<Pages page="a" />);
      const bar = container.querySelector('.scrollbar') as HTMLElement;
      // scrolled and pointed at, then a click elsewhere shows the other page within the time the bar lingers
      const a = getByTestId('a');
      a.scrollTop = 300;
      fireEvent.scroll(a);
      fireEvent.pointerEnter(a);
      expect(bar.classList.contains('active')).toBe(true);
      expect(bar.classList.contains('hover')).toBe(true);
      rerender(<Pages page="b" />);
      expect(container.querySelector('[data-testid="a"]')).toBeNull();
      expect(bar.classList.contains('active')).toBe(false);
      expect(bar.classList.contains('hover')).toBe(false);
      // the new page's own scrolling shows it as before
      const b = getByTestId('b');
      b.scrollTop = 100;
      fireEvent.scroll(b);
      expect(bar.classList.contains('active')).toBe(true);
    } finally { restore(); }
  });
});
