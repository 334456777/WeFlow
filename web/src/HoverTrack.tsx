// 会滑动的悬停凹陷（#232）：一组可以点击的项（导航、列表）共用一个浅的凹陷。指针移到另一项时，
// 凹陷从原来那一项滑过去，而不是一个淡出、另一个淡入；凹陷不跟着指针本身移动。
// 指针进入这一组时凹陷出现在指针所在的项上，离开时淡出。显示出来的键盘焦点也会移动它。
import { useEffect, useRef } from 'react';

/** 滚动后各行在指针下面移动：滚动后这么长时间内，凹陷直接跳到位而不滑动。 */
const SCROLL_QUIET_MS = 150;

/** 悬停凹陷。放在这一组（它的父元素）的最前面，按父元素定位；凹陷随这一组的项一起滚动。 */
export function HoverTrack({ items, quiet = '.selected, .active' }: {
  /** 这一组的项，写成选择器。 */
  items: string;
  /** 已经凹下去的项（选中的项）：凹陷在它们上面淡出，不让阴影叠成两层。 */
  quiet?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const hollow = ref.current;
    const group = hollow?.parentElement;
    if (!group || !hollow) return;
    group.classList.add('hover-track-host');
    let current: HTMLElement | null = null;
    let hovered = false;
    let scrolledAt = 0;

    const place = (item: HTMLElement, slide: boolean) => {
      // 先读完所有的值再写，免得中间页面重新排版（#961）
      const box = group.getBoundingClientRect();
      const at = item.getBoundingClientRect();
      const x = at.left - box.left - group.clientLeft + group.scrollLeft;
      const y = at.top - box.top - group.clientTop + group.scrollTop;
      // 这一项的轮廓，不论皮肤把它画成什么样（#831）：圆角、裁剪（在这一项上，
      // 或者在带外形层的皮肤下的外形层上，见 styles/roles.css）和旋转
      const shape = getComputedStyle(item);
      const clip = shape.clipPath && shape.clipPath !== 'none' ? shape.clipPath : getComputedStyle(item, '::before').clipPath;
      const radius = shape.borderRadius;
      const rotate = shape.rotate ?? '';
      if (!slide) hollow.style.transition = 'none';
      hollow.style.width = at.width + 'px';
      hollow.style.height = at.height + 'px';
      // 用 `translate` 而不是 `transform`：它在 `scale` 之前生效，所以按下时凹陷绕自己的中心缩小
      hollow.style.translate = `${x}px ${y}px`;
      hollow.style.borderRadius = radius;
      hollow.style.clipPath = clip ?? '';
      hollow.style.rotate = rotate;
      // 立即到位：恢复过渡之前，先在没有过渡的状态下算出样式。算出凹陷自己的样式就够了
      // （读一个不需要排版就能读的属性），不需要重新排版。
      if (!slide) { void getComputedStyle(hollow).opacity; hollow.style.transition = ''; }
    };
    const show = (item: HTMLElement) => {
      // 只有正在显示时才从原处滑过去；从外面进来（或者刚滚动过）时直接出现在那一项上
      const slide = (hollow.dataset.state === 'on' || hollow.dataset.state === 'quiet') && Date.now() - scrolledAt > SCROLL_QUIET_MS;
      place(item, slide);
      current = item;
      hollow.dataset.state = item.matches(quiet) ? 'quiet' : 'on';
    };
    const hide = () => {
      current = null;
      release();
      if (hollow.dataset.state) hollow.dataset.state = 'off';
    };
    // 按住时，这一项下面的凹陷自己变成按下的样子（并随这一项缩小），而不是这一项在凹陷上面再凹下去：
    // 只有一个变深的凹陷，不会同时出现两层阴影。
    const press = (event: PointerEvent) => {
      if (event.pointerType === 'touch' || event.button !== 0) return;
      const item = itemOf(event.target);
      if (item && item === current && !item.matches(quiet)) hollow.dataset.pressed = '';
    };
    const release = () => { delete hollow.dataset.pressed; };
    const itemOf = (target: EventTarget | null) => {
      const item = target instanceof Element ? target.closest<HTMLElement>(items) : null;
      return item && group.contains(item) ? item : null;
    };

    const over = (event: PointerEvent) => {
      if (event.pointerType === 'touch') return;
      hovered = true;
      // 指针在两项之间的空隙里时凹陷留在原处，这样到下一项时仍然滑过去
      const item = itemOf(event.target);
      if (item && item !== current) { release(); show(item); }
    };
    const leave = () => {
      hovered = false;
      const focused = itemOf(document.activeElement);
      if (focused?.matches(':focus-visible')) show(focused);
      else hide();
    };
    const focusIn = (event: FocusEvent) => {
      const item = itemOf(event.target);
      if (item?.matches(':focus-visible') && !hovered) show(item);
    };
    const focusOut = () => { if (!hovered) hide(); };
    const scrolled = () => { scrolledAt = Date.now(); };
    // 这一项变了（这一行换成了另一个会话、侧栏折叠）：立即跟过去
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => { if (current?.isConnected) place(current, false); else if (current) hide(); });
    resize?.observe(group);
    const changed = new MutationObserver(() => {
      if (!current) return;
      const state = !current.isConnected ? 'off' : current.matches(quiet) ? 'quiet' : 'on';
      if (hollow.dataset.state !== state) hollow.dataset.state = state;
    });
    changed.observe(group, { subtree: true, attributes: true, attributeFilter: ['class', 'aria-pressed', 'aria-current'], childList: true });

    group.addEventListener('pointerover', over);
    group.addEventListener('pointerleave', leave);
    group.addEventListener('pointerdown', press);
    window.addEventListener('pointerup', release);
    window.addEventListener('pointercancel', release);
    group.addEventListener('focusin', focusIn);
    group.addEventListener('focusout', focusOut);
    window.addEventListener('scroll', scrolled, { capture: true, passive: true });
    return () => {
      group.classList.remove('hover-track-host');
      group.removeEventListener('pointerover', over);
      group.removeEventListener('pointerleave', leave);
      group.removeEventListener('pointerdown', press);
      window.removeEventListener('pointerup', release);
      window.removeEventListener('pointercancel', release);
      group.removeEventListener('focusin', focusIn);
      group.removeEventListener('focusout', focusOut);
      window.removeEventListener('scroll', scrolled, { capture: true });
      resize?.disconnect();
      changed.disconnect();
    };
  }, [items, quiet]);

  return <div ref={ref} className="hover-hollow" aria-hidden="true" />;
}
