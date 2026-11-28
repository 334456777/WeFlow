// 预览里可以缩放和拖动的图片（#194）：滚轮以指针为中心缩放，倍数随转动的幅度增大；按住移动超过 4px 才开始拖动，
// 点击时手抖一下不会移动图片；双击在指针处放大到 2.5 倍，再双击复原。图片始终不离开框。
import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import type { CSSProperties } from 'react';

/** 最小的缩放倍数：图片正好放进框里。 */
export const MIN_SCALE = 1;
/** 最大的缩放倍数。 */
export const MAX_SCALE = 8;
/** 滚轮转一格（`deltaY` 为 100）缩放的程度：e^(0.15) ≈ 1.16 倍。 */
const WHEEL = 0.0015;
/** 按下后移动多少像素才算拖动。 */
const DEAD_ZONE = 4;
const DOUBLE = 2.5;

/** 图片的显示方式：缩放倍数，以及相对框中心的偏移（像素）。 */
export interface View { scale: number; x: number; y: number }
/** 图片正好放进框里、居中。 */
export const FIT: View = { scale: 1, x: 0, y: 0 };

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

/**
 * 以点（`px`, `py`，从框中心量起）为中心把 `view` 缩放到 `scale`，再限制在框内：
 * 1 倍时为 `width` × `height` 的图片，每个方向最多移动它变大部分的一半。
 */
export function zoomAt(view: View, scale: number, px: number, py: number, width: number, height: number): View {
  const next = clamp(scale, MIN_SCALE, MAX_SCALE);
  if (next === MIN_SCALE) return FIT;
  const k = next / view.scale;
  return keep({ scale: next, x: px - (px - view.x) * k, y: py - (py - view.y) * k }, width, height);
}

/** 把 `view` 移回框内；图片在 1 倍时为 `width` × `height`。 */
export function keep(view: View, width: number, height: number): View {
  const roomX = (width * view.scale - width) / 2;
  const roomY = (height * view.scale - height) / 2;
  return { scale: view.scale, x: clamp(view.x, -roomX, roomX), y: clamp(view.y, -roomY, roomY) };
}

/** 预览的按键（`+`、`-`、`0`）对图片的操作：以框的中心缩放，或者恢复到正好放进框里。 */
export interface ZoomControls { zoomBy(factor: number): void; reset(): void }

/** 可缩放的图片。`sized`：预览事先知道图片的形状时，图片的大小和加载前显示的内容（#194）。 */
export const ZoomImage = forwardRef<ZoomControls, { src: string; alt: string; onLoad?(): void; onError(): void; sized?: CSSProperties }>(function ZoomImage({ src, alt, onLoad, onError, sized }, ref) {
  const [view, setView] = useState<View>(FIT);
  const frame = useRef<HTMLDivElement>(null);
  const picture = useRef<HTMLImageElement>(null);
  const current = useRef(view);
  current.current = view;
  const size = () => ({ width: picture.current?.offsetWidth ?? 0, height: picture.current?.offsetHeight ?? 0 });

  useImperativeHandle(ref, () => ({
    zoomBy: factor => { const { width, height } = size(); setView(v => zoomAt(v, v.scale * factor, 0, 0, width, height)); },
    reset: () => setView(FIT),
  }), []);

  // 直接监听滚轮：React 的处理函数是被动的（passive），无法阻止页面滚动
  useEffect(() => {
    const box = frame.current;
    if (!box) return;
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      const rect = box.getBoundingClientRect();
      const { width, height } = size();
      const delta = event.deltaMode === 1 ? event.deltaY * 33 : event.deltaY;
      const v = current.current;
      setView(zoomAt(v, v.scale * Math.exp(-delta * WHEEL), event.clientX - rect.left - rect.width / 2, event.clientY - rect.top - rect.height / 2, width, height));
    };
    box.addEventListener('wheel', wheel, { passive: false });
    return () => box.removeEventListener('wheel', wheel);
  }, []);

  const press = useRef<{ x: number; y: number; from: View; moved: boolean; id: number } | null>(null);
  return <div ref={frame} className={'zoom-frame' + (view.scale > 1 ? ' zoomed' : '')}
    onPointerDown={event => {
      if (event.button !== 0 || current.current.scale === 1) return;
      press.current = { x: event.clientX, y: event.clientY, from: current.current, moved: false, id: event.pointerId };
    }}
    onPointerMove={event => {
      const p = press.current;
      if (!p || p.id !== event.pointerId) return;
      const dx = event.clientX - p.x;
      const dy = event.clientY - p.y;
      if (!p.moved && Math.hypot(dx, dy) < DEAD_ZONE) return;
      if (!p.moved) { p.moved = true; event.currentTarget.setPointerCapture?.(event.pointerId); }
      const { width, height } = size();
      setView(keep({ scale: p.from.scale, x: p.from.x + dx, y: p.from.y + dy }, width, height));
    }}
    onPointerUp={() => { press.current = null; }}
    onPointerCancel={() => { press.current = null; }}
    onDoubleClick={event => {
      const rect = event.currentTarget.getBoundingClientRect();
      const { width, height } = size();
      setView(v => (v.scale > 1 ? FIT : zoomAt(v, DOUBLE, event.clientX - rect.left - rect.width / 2, event.clientY - rect.top - rect.height / 2, width, height)));
    }}>
    <img ref={picture} src={src} alt={alt} draggable={false} onLoad={onLoad} onError={onError}
      style={{ ...sized, transform: 'translate(' + view.x + 'px, ' + view.y + 'px) scale(' + view.scale + ')' }} />
  </div>;
});
