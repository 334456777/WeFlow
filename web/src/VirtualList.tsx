// 只渲染看得见的行的列表（#159）：各行等高，绝对定位在一个与全部行一样高的占位元素里，
// 这样几千个会话滚动时也不会有几千个元素。
import { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from 'react';
import type { CSSProperties, ForwardedRef, HTMLAttributes, ReactNode, Ref } from 'react';
import { HoverTrack } from './HoverTrack';
import { Scrollbar } from './Scrollbar';

/** 列表的 `ref` 提供的操作。 */
export interface VirtualListHandle {
  /** 滚动到刚好让这一行进入视野。 */
  scrollToIndex(index: number): void;
}

interface Props<T> extends Omit<HTMLAttributes<HTMLDivElement>, 'children'> {
  items: readonly T[];
  /** 相邻两行之间的距离，包括间隙。 */
  rowHeight: number;
  keyOf(item: T): string;
  renderRow(item: T, index: number): ReactNode;
  /** 视野上下边缘之外多渲染的行数，快速滚动时不会露出空白。 */
  overscan?: number;
  /** 第一行上方和最后一行下方的空白。 */
  padding?: number;
  /** 各行里可以点击的项，写成选择器：悬停凹陷在它们之间滑动（#232）。 */
  track?: string;
}

/** 列表还不知道自己的高度时（第一次绘制，或者没有排版的测试）渲染的行数。 */
const FALLBACK_ROWS = 20;

/**
 * 由行的键（FNV-1a 散列）算出的 [0, 1) 之间的数，行里元素的种子从它开始（styles/roles.css，#831）：
 * 列表滚动时这些元素在可见行中的位置会变，而它们显示的项不变，所以种子跟着项走。
 */
export function keySeed(key: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < key.length; i++) hash = Math.imul(hash ^ key.charCodeAt(i), 0x01000193);
  return (hash >>> 0) / 2 ** 32;
}

/** 渲染的行：第一行的序号，以及填满可见高度要多少行（列表还不知道高度时为 0）。 */
interface Range { first: number; visible: number }

/** 列表滚动到 `top`、可见高度为 `height` 时要渲染的行。 */
function rangeOf(top: number, height: number, { rowHeight, padding, overscan }: { rowHeight: number; padding: number; overscan: number }): Range {
  return { first: Math.max(0, Math.floor((top - padding) / rowHeight) - overscan), visible: height ? Math.ceil(height / rowHeight) : 0 };
}

function VirtualListInner<T>(
  { items, rowHeight, keyOf, renderRow, overscan = 6, padding = 0, track, className, ...rest }: Props<T>,
  ref: ForwardedRef<VirtualListHandle>,
) {
  const scroller = useRef<HTMLDivElement>(null);
  // 只有要渲染的行变了才重新渲染，而不是滚动的每一帧都渲染：滚动本身就会移动已经渲染的行（#961）。
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

  // 不论滚动事件来得多快，每帧只更新一次。
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

/** `items` 的列表，进入视野的行由 `renderRow` 画出。 */
export const VirtualList = forwardRef(VirtualListInner) as <T>(
  props: Props<T> & { ref?: Ref<VirtualListHandle> },
) => ReturnType<typeof VirtualListInner>;
