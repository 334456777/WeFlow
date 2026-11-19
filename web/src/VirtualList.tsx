// A list that renders only the rows in sight (#159): rows of one height, placed absolutely in a spacer as tall as
// all of them, so thousands of conversations scroll without thousands of elements.
import { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from 'react';
import type { CSSProperties, ForwardedRef, HTMLAttributes, ReactNode, Ref } from 'react';
import { HoverTrack } from './HoverTrack';
import { Scrollbar } from './Scrollbar';

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
  /** The rows' clickable items, as a selector: a hover hollow slides between them (#232). */
  track?: string;
}

/** Rows drawn before the list knows its height (the first paint, or a test without layout). */
const FALLBACK_ROWS = 20;

/** A number in [0, 1) of a row's key (FNV-1a), which the seed of the elements in the row starts from (styles/roles.css,
 * #831): their place among the rows in sight changes as the list scrolls, the item they show does not. */
export function keySeed(key: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < key.length; i++) hash = Math.imul(hash ^ key.charCodeAt(i), 0x01000193);
  return (hash >>> 0) / 2 ** 32;
}

/** The rows drawn: the first, and how many fill the height in sight (0 before the list knows its height). */
interface Range { first: number; visible: number }

/** The rows to draw for the list scrolled to `top` in a window `height` high. */
function rangeOf(top: number, height: number, { rowHeight, padding, overscan }: { rowHeight: number; padding: number; overscan: number }): Range {
  return { first: Math.max(0, Math.floor((top - padding) / rowHeight) - overscan), visible: height ? Math.ceil(height / rowHeight) : 0 };
}

function VirtualListInner<T>(
  { items, rowHeight, keyOf, renderRow, overscan = 6, padding = 0, track, className, ...rest }: Props<T>,
  ref: ForwardedRef<VirtualListHandle>,
) {
  const scroller = useRef<HTMLDivElement>(null);
  // The list draws again only when the rows to draw change, not at every frame of a scroll, which moves the rows
  // drawn already without them (#961).
  const [range, setRange] = useState<Range>({ first: 0, visible: 0 });

  const measure = useCallback(() => {
    const el = scroller.current;
    if (!el) return;
    const next = rangeOf(el.scrollTop, el.clientHeight, { rowHeight, padding, overscan });
    setRange(old => (old.first === next.first && old.visible === next.visible ? old : next));
  }, [rowHeight, padding, overscan]);

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

  const visible = range.visible || FALLBACK_ROWS;
  const first = range.first;
  const last = Math.min(items.length, first + visible + 2 * overscan + 1);
  const rows: ReactNode[] = [];
  for (let i = first; i < last; i++) {
    const item = items[i];
    const key = keyOf(item);
    const style = { transform: `translateY(${padding + i * rowHeight}px)`, height: rowHeight, '--key-seed': String(keySeed(key)) } as CSSProperties;
    rows.push(<div key={key} className="virtual-row" style={style}>{renderRow(item, i)}</div>);
  }
  return <><div ref={scroller} className={'virtual-list ' + (className ?? '')} onScroll={onScroll} {...rest}>
    <div className="virtual-space" style={{ height: 2 * padding + items.length * rowHeight }}>{track && <HoverTrack items={track} />}{rows}</div>
  </div><Scrollbar target={scroller} /></>;
}

export const VirtualList = forwardRef(VirtualListInner) as <T>(
  props: Props<T> & { ref?: Ref<VirtualListHandle> },
) => ReturnType<typeof VirtualListInner>;
