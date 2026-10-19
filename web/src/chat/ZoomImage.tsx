// A picture in the preview that zooms and pans (#194): the wheel zooms by a factor that grows with how far it turns,
// around the pointer; a drag pans once it has moved 4px, so a click that wobbles does not; a double click goes to
// 2.5× at the pointer and back. The picture never leaves the frame.
import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import type { CSSProperties } from 'react';

export const MIN_SCALE = 1;
export const MAX_SCALE = 8;
/** How much one wheel step (100 deltaY) zooms: e^(0.15) ≈ 1.16×. */
const WHEEL = 0.0015;
/** Movement before a press becomes a drag. */
const DEAD_ZONE = 4;
const DOUBLE = 2.5;

export interface View { scale: number; x: number; y: number }
export const FIT: View = { scale: 1, x: 0, y: 0 };

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

/**
 * `view` zoomed to `scale` around the point (`px`, `py`), measured from the frame's centre, then kept inside the frame:
 * a picture `width` × `height` at scale 1 can move half of what it grew each way.
 */
export function zoomAt(view: View, scale: number, px: number, py: number, width: number, height: number): View {
  const next = clamp(scale, MIN_SCALE, MAX_SCALE);
  if (next === MIN_SCALE) return FIT;
  const k = next / view.scale;
  return keep({ scale: next, x: px - (px - view.x) * k, y: py - (py - view.y) * k }, width, height);
}

export function keep(view: View, width: number, height: number): View {
  const roomX = (width * view.scale - width) / 2;
  const roomY = (height * view.scale - height) / 2;
  return { scale: view.scale, x: clamp(view.x, -roomX, roomX), y: clamp(view.y, -roomY, roomY) };
}

export interface ZoomControls { zoomBy(factor: number): void; reset(): void }

/** `sized`: the picture's size and what shows until it loads, when the preview knows its shape beforehand (#194). */
export const ZoomImage = forwardRef<ZoomControls, { src: string; alt: string; onError(): void; sized?: CSSProperties }>(function ZoomImage({ src, alt, onError, sized }, ref) {
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

  // the wheel is listened to directly: React's handler is passive and could not keep the page from scrolling
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
    <img ref={picture} src={src} alt={alt} draggable={false} onError={onError}
      style={{ ...sized, transform: 'translate(' + view.x + 'px, ' + view.y + 'px) scale(' + view.scale + ')' }} />
  </div>;
});
