// 锚定在控件上的浮层（#224）：从控件处展开，保持在屏幕内，点击外面或按 Esc 时关闭。
// 它放在 `<body>` 里，所以卡片的 overflow 不会把它裁掉。
//
// 下方放得下时在控件下方打开，只有上方放得下时在上方打开。上下都放不下时选空间大的一边，在浮层里滚动；
// 屏幕比浮层加两侧留白还窄时先缩小留白，再横向滚动（#239）。控件滚出视野（窗口或所在列表的视野）时浮层隐藏，
// 控件回到视野时再显示。
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { CSSProperties, ReactNode, RefObject } from 'react';
import { createPortal } from 'react-dom';
import { usePresence } from './motion';

/** 与锚点的距离，以及与窗口边缘至少保留的距离。 */
const GAP = 8;
const EDGE = 16;

interface Place { left: number; top: number; origin: string; maxWidth?: number; maxHeight?: number; hidden: boolean }

/** 锚点没有被窗口、或者会滚动和裁剪的祖先元素挡住的部分。 */
export function visibleBox(anchor: HTMLElement): { top: number; bottom: number } {
  const box = anchor.getBoundingClientRect();
  let top = Math.max(box.top, 0);
  let bottom = Math.min(box.bottom, window.innerHeight);
  for (let p = anchor.parentElement; p && p !== document.body; p = p.parentElement) {
    if (getComputedStyle(p).overflowY === 'visible') continue;
    const clip = p.getBoundingClientRect();
    top = Math.max(top, clip.top);
    bottom = Math.min(bottom, clip.bottom);
  }
  return { top, bottom };
}

/** 自然尺寸为 `width` × `height` 的浮层在锚点旁边的位置。 */
export function placeOf(anchor: DOMRect, width: number, height: number, hidden = false): Place {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const edge = Math.max(0, Math.min(EDGE, (vw - width) / 2));
  const maxWidth = width > vw - 2 * edge ? vw - 2 * edge : undefined;
  const left = Math.max(edge, Math.min(anchor.left, vw - edge - Math.min(width, vw)));
  const roomBelow = vh - EDGE - anchor.bottom - GAP;
  const roomAbove = anchor.top - GAP - EDGE;
  const below = height <= roomBelow || (height > roomAbove && roomBelow >= roomAbove);
  const room = Math.max(0, below ? roomBelow : roomAbove);
  const maxHeight = height > room ? room : undefined;
  const top = below ? anchor.bottom + GAP : anchor.top - GAP - Math.min(height, room);
  // 从锚点的中间展开
  const origin = Math.round(anchor.left + anchor.width / 2 - left) + 'px ' + (below ? 'top' : 'bottom');
  return { left, top, origin, maxWidth, maxHeight, hidden };
}

function styleOf(place: Place | null): CSSProperties {
  // 量好尺寸之前（第一次绘制之前）先放在屏幕外，在那里已经可以获得焦点
  if (!place) return { left: -9999, top: 0 };
  const scrolls = place.maxHeight !== undefined || place.maxWidth !== undefined;
  return {
    left: place.left, top: place.top, transformOrigin: place.origin, maxWidth: place.maxWidth, maxHeight: place.maxHeight,
    overflow: scrolls ? 'auto' : undefined, visibility: place.hidden ? 'hidden' : undefined,
  };
}

/** `open` 时把 `children` 放进锚定在 `anchor` 上的浮层。 */
export function Popover({ anchor, open, onClose, label, className, children }: {
  anchor: RefObject<HTMLElement | null>;
  open: boolean;
  /** 按 Esc（`escape`）关闭时把焦点还给控件；点击外面（`outside`）关闭时焦点留在点击的地方。 */
  onClose: (reason: 'escape' | 'outside') => void;
  label: string;
  className?: string;
  children: ReactNode;
}) {
  const shown = usePresence(open || null);
  const ref = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<Place | null>(null);
  const close = useRef(onClose);
  close.current = onClose;

  useLayoutEffect(() => {
    if (!shown) { setPlace(null); return; }
    const update = () => {
      const control = anchor.current;
      const layer = ref.current;
      if (!control || !layer) return;
      const seen = visibleBox(control);
      // 自然尺寸，不论是否在滚动：内容加内边距（浮层没有边框）
      setPlace(placeOf(control.getBoundingClientRect(), layer.scrollWidth, layer.scrollHeight, seen.bottom < seen.top));
    };
    update();
    // 操作控制台和页面在它下面滚动：跟着锚点移动
    window.addEventListener('scroll', update, true);
    window.addEventListener('resize', update);
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(update);
    if (ref.current) observer?.observe(ref.current);
    return () => {
      window.removeEventListener('scroll', update, true);
      window.removeEventListener('resize', update);
      observer?.disconnect();
    };
  }, [Boolean(shown), anchor]);

  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!ref.current?.contains(target) && !anchor.current?.contains(target)) close.current('outside');
    };
    // 焦点不在任何元素上（在 `<body>` 上）时按 Esc 也关闭；焦点在浮层里时由下面浮层自己的处理函数关闭
    const escape = (event: KeyboardEvent) => {
      const at = document.activeElement;
      if (event.key === 'Escape' && (!at || at === document.body)) close.current('escape');
    };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('keydown', escape);
    };
  }, [open, anchor]);

  if (!shown) return null;
  return createPortal(
    <div ref={ref} role="dialog" aria-label={label} className={'popover' + (className ? ' ' + className : '')}
      data-state={shown.closing ? 'closing' : 'open'} inert={shown.closing}
      style={styleOf(place)}
      onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); close.current('escape'); } }}>
      {children}
    </div>,
    document.body,
  );
}
