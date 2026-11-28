// 页面自己画在滚动区域上的滚动条（`.scroll-area` 隐藏了浏览器的滚动条）：它不占宽度，出现时内容不会移动，
// 样子也与页面其他部分一致（皮肤的滚动条角色，#828）。紧跟在滚动区域后面渲染，放在同一个定位的父元素里，
// 它自己沿滚动区域的右边缘定位。
// 它还在滚动区域的上下边缘做渐隐，表示那个方向还能滚动（#194、#831）：
// 一个与滚动区域大小、圆角和裁剪都相同的框盖在区域上，上下边缘各有一条带子，app.css 用区域下面的底色画它们，
// 那一边还有内容时才显示（`shown`）。
import { useEffect, useRef } from 'react';
import type { RefObject } from 'react';

/** 滚动条与滚动区域边缘的距离。 */
const INSET = 4;
const MIN_THUMB = 24;
/** 最后一次滚动后滚动条继续显示的时长。 */
const LINGER_MS = 900;
/**
 * 用户开始拖动滑块时发给滚动区域的事件。滑块不在区域里面，区域自己的指针事件看不到用户在滚动
 * （聊天窗口收到它时就不再停在底部）。
 */
export const SCROLL_INTENT = 'weflow:scroll-intent';

/** 指针是否在 `element` 上；不能查询 `:hover` 时为 false。 */
function hovered(element: Element) {
  try { return element.matches(':hover'); } catch { return false; }
}

/** 滚动区域 `target` 的滚动条和边缘渐隐。 */
export function Scrollbar({ target }: { target: RefObject<HTMLElement | null> }) {
  const track = useRef<HTMLDivElement>(null);
  const thumb = useRef<HTMLDivElement>(null);
  const fades = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const area = target.current;
    const bar = track.current;
    const knob = thumb.current;
    if (!area || !bar || !knob) return;
    let travel = 0;
    let room = 0;
    let linger = 0;

    // 滚动的每一帧、面板变大变小（侧栏折叠）的每一帧都会运行：先读完所有值再写，这样每帧只排版一次，
    // 不会每读一次就排版一次（#961）；样式只在变化时才设置。
    const set = (element: HTMLElement, property: 'left' | 'top' | 'width' | 'height' | 'borderRadius' | 'clipPath' | 'transform', value: string) => {
      if (element.style[property] !== value) element.style[property] = value;
    };
    const place = () => {
      const { scrollHeight, clientHeight, scrollTop } = area;
      room = scrollHeight - clientHeight;
      const overflow = room > 1 && clientHeight > 0;
      const left = area.offsetLeft + area.clientLeft;
      const top = area.offsetTop + area.clientTop;
      const width = area.clientWidth;
      // 滚动条自己的宽度，取自它的样式；没有可滚动的内容时它在页面上不占宽度
      const barWidth = overflow ? Number.parseFloat(getComputedStyle(bar).width) || 0 : 0;
      // 滚动区域的外形，不论皮肤把它画成什么样：圆角和裁剪
      const shape = overflow && fades.current ? getComputedStyle(area) : null;
      const radius = shape?.borderRadius ?? '';
      const clip = shape?.clipPath ?? '';

      bar.classList.toggle('scrollable', overflow);
      const frame = fades.current;
      if (frame) {
        const [above, below] = frame.firstElementChild!.children;
        frame.classList.toggle('scrollable', overflow);
        above.classList.toggle('shown', overflow && scrollTop > 1);
        below.classList.toggle('shown', overflow && scrollTop < room - 1);
        if (overflow) {
          set(frame, 'left', left + 'px');
          set(frame, 'top', top + 'px');
          set(frame, 'width', width + 'px');
          set(frame, 'height', clientHeight + 'px');
          set(frame, 'borderRadius', radius);
          set(frame, 'clipPath', clip);
        }
      }
      if (!overflow) return;
      const height = clientHeight - 2 * INSET;
      set(bar, 'top', top + INSET + 'px');
      set(bar, 'left', left + width - barWidth + 'px');
      set(bar, 'height', height + 'px');
      const size = Math.min(height, Math.max(MIN_THUMB, (height * clientHeight) / scrollHeight));
      travel = height - size;
      set(knob, 'height', size + 'px');
      set(knob, 'transform', 'translateY(' + (travel * Math.min(1, scrollTop / room)) + 'px)');
    };
    const show = () => {
      // 没有这个类时才加：重复加上，对监视类名的一方来说仍然算一次变化
      if (!bar.classList.contains('active')) bar.classList.add('active');
      window.clearTimeout(linger);
      linger = window.setTimeout(() => bar.classList.remove('active'), LINGER_MS);
    };
    const onScroll = () => { place(); show(); };
    // 指针在区域或滑块上时显示；滑块不在区域的元素里面。
    const enter = () => { place(); bar.classList.add('hover'); };
    const leave = (event: PointerEvent) => {
      const to = event.relatedTarget as Node | null;
      if (to !== knob && !(to && area.contains(to))) bar.classList.remove('hover');
    };

    // 内容会变大变小（卡片展开、新的行到达）而区域本身的大小不变：所以监视它的子元素。
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(place);
    const watch = () => {
      if (resize) {
        resize.disconnect();
        resize.observe(area);
        for (const child of Array.from(area.children)) resize.observe(child);
      }
      place();
    };
    const children = new MutationObserver(watch);
    children.observe(area, { childList: true });
    watch();
    area.addEventListener('scroll', onScroll, { passive: true });
    area.addEventListener('pointerenter', enter);
    area.addEventListener('pointerleave', leave);
    // 指针已经在新的区域上时（同一个面板换了页面，#836），区域收不到 `pointerenter`
    if (hovered(area)) bar.classList.add('hover');

    // 拖动滑块时按比例滚动。只有滑块接收指针：滑块旁边的轨道会盖住各行和气泡的右边缘。
    let drag: { y: number; top: number } | null = null;
    const down = (event: PointerEvent) => {
      if (event.button !== 0) return;
      event.preventDefault();
      drag = { y: event.clientY, top: area.scrollTop };
      area.dispatchEvent(new Event(SCROLL_INTENT));
      knob.setPointerCapture?.(event.pointerId);
      bar.classList.add('dragging');
    };
    const move = (event: PointerEvent) => {
      if (!drag || !travel) return;
      const top = drag.top + ((event.clientY - drag.y) * room) / travel;
      // 立即滚动：平滑滚动的区域（聊天窗口）会落在指针后面
      if (area.scrollTo) area.scrollTo({ top, behavior: 'instant' }); else area.scrollTop = top;
    };
    const up = () => { drag = null; bar.classList.remove('dragging'); show(); };
    knob.addEventListener('pointerdown', down);
    knob.addEventListener('pointerleave', leave);
    knob.addEventListener('pointermove', move);
    knob.addEventListener('pointerup', up);
    knob.addEventListener('pointercancel', up);

    return () => {
      resize?.disconnect();
      children.disconnect();
      window.clearTimeout(linger);
      // 面板换页时滚动条比滚动区域留得更久（#836）：为这个区域显示的状态在这里结束，
      // 因为本来会把它隐藏的定时器已经清掉了
      bar.classList.remove('active', 'hover', 'dragging');
      area.removeEventListener('scroll', onScroll);
      area.removeEventListener('pointerenter', enter);
      area.removeEventListener('pointerleave', leave);
      knob.removeEventListener('pointerdown', down);
      knob.removeEventListener('pointerleave', leave);
      knob.removeEventListener('pointermove', move);
      knob.removeEventListener('pointerup', up);
      knob.removeEventListener('pointercancel', up);
    };
  }, [target]);

  return <>
    <div ref={fades} className="scroll-fades" aria-hidden="true">
      <div><div className="scroll-fade top" /><div className="scroll-fade bottom" /></div>
    </div>
    <div ref={track} className="scrollbar" aria-hidden="true"><div ref={thumb} className="scrollbar-thumb" /></div>
  </>;
}
