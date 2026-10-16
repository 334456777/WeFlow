// A list that renders only the rows in sight (#159): rows of one height, placed absolutely in a spacer as tall as
// all of them, so thousands of conversations scroll without thousands of elements.
import { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from 'react';
import type { ForwardedRef, HTMLAttributes, ReactNode, Ref } from 'react';

export interface VirtualListHandle {
  /** Scrolls just enough to bring the row into sight. */
  scrollToIndex(index: number): void;
}

interface Props<T> extends Omit<HTMLAttributes<HTMLDivElement>, 'children'> {
  items: readonly T[];
  /** The distance from one row to the next, gap included. */
  rowHeight: number;
  keyOf(item: T): string;
  renderRow(item: T, index: number): ReactNode;
  /** Rows kept beyond each edge, so a quick scroll does not show blank space. */
  overscan?: number;
  /** Space above the first row and below the last. */
  padding?: number;
}

/** Rows drawn before the list knows its height (the first paint, or a test without layout). */
const FALLBACK_ROWS = 20;

function VirtualListInner<T>(
  { items, rowHeight, keyOf, renderRow, overscan = 6, padding = 0, className, ...rest }: Props<T>,
  ref: ForwardedRef<VirtualListHandle>,
) {
  const scroller = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 0 });

  const measure = useCallback(() => {
    const el = scroller.current;
    if (el) setView(old => (old.top === el.scrollTop && old.height === el.clientHeight ? old : { top: el.scrollTop, height: el.clientHeight }));
  }, []);

  useLayoutEffect(measure, [measure]);
  useEffect(() => {
    const el = scroller.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [measure]);

  // One update per frame however fast the scroll events come.
  const frame = useRef(0);
  const onScroll = useCallback(() => {
    if (frame.current) return;
    frame.current = requestAnimationFrame(() => { frame.current = 0; measure(); });
  }, [measure]);
  useEffect(() => () => cancelAnimationFrame(frame.current), []);

  useImperativeHandle(ref, () => ({
    scrollToIndex(index) {
      const el = scroller.current;
      if (!el) return;
      const top = padding + index * rowHeight;
      const bottom = top + rowHeight;
      if (top < el.scrollTop + padding) el.scrollTop = top - padding;
      else if (bottom > el.scrollTop + el.clientHeight - padding) el.scrollTop = bottom - el.clientHeight + padding;
      measure();
    },
  }), [padding, rowHeight, measure]);

  const visible = view.height ? Math.ceil(view.height / rowHeight) : FALLBACK_ROWS;
  const first = Math.max(0, Math.floor((view.top - padding) / rowHeight) - overscan);
  const last = Math.min(items.length, first + visible + 2 * overscan + 1);
  const rows: ReactNode[] = [];
  for (let i = first; i < last; i++) {
    const item = items[i];
    rows.push(<div key={keyOf(item)} className="virtual-row" style={{ transform: `translateY(${padding + i * rowHeight}px)`, height: rowHeight }}>{renderRow(item, i)}</div>);
  }
  return <div ref={scroller} className={'virtual-list ' + (className ?? '')} onScroll={onScroll} {...rest}>
    <div className="virtual-space" style={{ height: 2 * padding + items.length * rowHeight }}>{rows}</div>
  </div>;
}

export const VirtualList = forwardRef(VirtualListInner) as <T>(
  props: Props<T> & { ref?: Ref<VirtualListHandle> },
) => ReturnType<typeof VirtualListInner>;
