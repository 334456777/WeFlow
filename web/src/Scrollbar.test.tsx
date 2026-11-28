import { cleanup, fireEvent, render } from '@testing-library/react';
import { useRef } from 'react';
import { afterEach, describe, expect, it, vi } from '#test';
import { SCROLL_INTENT, Scrollbar } from './Scrollbar';

afterEach(cleanup);

function Area() {
  const ref = useRef<HTMLDivElement>(null);
  return <><div ref={ref} className="scroll-area" data-testid="area"><p>content</p></div><Scrollbar target={ref} /></>;
}

/** 两个页面共用一个滚动条，与面板先显示一个页面、再换成另一个时保留滚动条一样（#836）。 */
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

// happy-dom 不排版：在滚动条量尺寸之前给区域设定大小。
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
      // 盖在区域上，与它一样高（测试里的区域没有宽度）
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
      // 轨道是区域两端各去掉 4px，即 400px；区域显示内容的十分之一，滑块也占轨道的十分之一。
      expect(bar.style.height).toBe('400px');
      expect(thumb.style.height).toBe('40px');

      area.scrollTop = 1836;
      fireEvent.scroll(area);
      expect(bar.classList.contains('active')).toBe(true);
      expect(thumb.style.transform).toBe('translateY(180px)');

      // 36px 是滑块可移动的 360px 的十分之一，所以滚动区域可滚动的 3672px 的十分之一；
      // 区域收到用户在滚动它的事件（滑块不在区域里面）
      const intent = vi.fn();
      area.addEventListener(SCROLL_INTENT, intent);
      fireEvent.pointerDown(thumb, { button: 0, clientY: 200, pointerId: 1 });
      expect(bar.classList.contains('dragging')).toBe(true);
      expect(intent).toHaveBeenCalledTimes(1);
      fireEvent.pointerMove(thumb, { clientY: 236, pointerId: 1 });
      expect(area.scrollTop).toBeCloseTo(1836 + 367.2);
      // 平滑滚动的区域要立即移动，否则会落在指针后面（#159）
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
      // 滚动并把指针移到区域上，然后在滚动条消失之前点了别处，换成另一个页面
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
      // 新页面自己滚动时，滚动条像前面一样显示
      const b = getByTestId('b');
      b.scrollTop = 100;
      fireEvent.scroll(b);
      expect(bar.classList.contains('active')).toBe(true);
    } finally { restore(); }
  });
});
