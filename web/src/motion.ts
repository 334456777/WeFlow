// 动画的辅助函数（#193）。时长和曲线定义在 styles/tokens.css 里，脚本也从那里读取，这样「减少动画」开关和
// `prefers-reduced-motion`（两者都把时长设为 0ms）对脚本同样生效。
import { useEffect, useState } from 'react';
import { turnPage } from './pageTurn';

/**
 * 把时长令牌和曲线令牌（如 `--duration-panel`、`--ease-snap`）换成 Web Animations API 接受的形式；
 * 减少动画或没有时长令牌时时长为 0，没有曲线令牌时曲线为 `ease`。
 */
export function timing(duration: string, easing: string, element: Element = document.documentElement): { duration: number; easing: string } {
  return { duration: durationMs(duration, element), easing: getComputedStyle(element).getPropertyValue(easing).trim() || 'ease' };
}

/**
 * 按 app.css 里 `content-in` 的方式让内容进入（#776）：从透明、低半个 `--enter-offset` 的位置开始，用 `--ease-out`
 * 在 `--duration-base` 内进入。用于不重新挂载的内容，如同一面板里的另一页、同一窗格里另一项的详情；
 * 减少动画时令牌为 0，不播放。
 */
export function contentIn(elements: Iterable<Element>, root: Element = document.documentElement): void {
  const options = timing('--duration-base', '--ease-out', root);
  if (!options.duration) return;
  const offset = getComputedStyle(root).getPropertyValue('--enter-offset').trim() || '0px';
  const frames = [{ opacity: 0, transform: `translateY(calc(.5 * ${offset}))` }, { opacity: 1, transform: 'none' }];
  for (const element of elements) element.animate?.(frames, options);
}

/** pageIn 在元素上播放的动画结束时的处理函数，它把元素的 `animation` 交还给样式表。 */
const pageInEnds = new WeakMap<Element, (event: Event) => void>();

/** `element` 是否正在运行 `event` 所指名称的动画：按 getAnimations() 列出的 CSS 动画判断。 */
function stillRuns(element: Element, event: Event): boolean {
  const name = (event as AnimationEvent).animationName;
  return Boolean(element.getAnimations?.().some(animation => 'animationName' in animation && animation.animationName === name && animation.playState === 'running'));
}

/**
 * 让另一个页面或页面里另一项的详情进入（#776、#832），并通知皮肤：记一次换页（pageTurn.ts），
 * 皮肤的装饰层（Backdrop.tsx）随之轮换；元素按皮肤自己的进入动画 `--page-in` 进入（一个 `animation`，
 * 关键帧在皮肤的样式表里），皮肤没有这个动画时按 contentIn 的方式进入。
 * 减少动画时什么都不做：没有动画，装饰层也不变。也不会有代码等它播放完（#780）。
 * `detail` 表示同一页面里换成另一项的详情：记一次换页，但贴在页面上的东西不变。
 * `turn: false` 表示进入的内容没有替换另一个页面，即详情从无到有（页面打开时选中第一项）。
 */
export function pageIn(elements: Iterable<Element>, { turn = true, detail = false, root = document.documentElement }: { turn?: boolean; detail?: boolean; root?: Element } = {}): void {
  if (!durationMs('--duration-base', root)) return;
  if (turn) turnPage({ detail });
  const own = getComputedStyle(root).getPropertyValue('--page-in').trim();
  if (!own || own === 'none') {
    contentIn(elements, root);
    return;
  }
  for (const element of elements) {
    if (!(element instanceof HTMLElement)) continue;
    const before = pageInEnds.get(element);
    if (before) { element.removeEventListener('animationend', before); element.removeEventListener('animationcancel', before); }
    const end = (event: Event) => {
      // 内部元素的动画结束事件也会冒泡到这里
      if (event.target !== element) return;
      // 动画在结束之前重新开始（见下面）时会先被取消，浏览器要到下一帧、动画已经重新运行时才发出取消事件：
      // 这不是正在运行的动画结束了。
      if (event.type === 'animationcancel' && stillRuns(element, event)) return;
      element.removeEventListener('animationend', end);
      element.removeEventListener('animationcancel', end);
      pageInEnds.delete(element);
      element.style.animation = '';
    };
    pageInEnds.set(element, end);
    element.addEventListener('animationend', end);
    element.addEventListener('animationcancel', end);
    // 动画结束之前又来一次换页时从头播放：先在去掉动画的状态下计算一次元素的样式，再把动画交还给它。
    // 读一个不需要布局就能读到的属性就足以计算样式，刚进入的页面因此不会在这里、在它的帧之前被提前布局（#961）。
    element.style.animation = 'none';
    void getComputedStyle(element).animationName;
    element.style.animation = own;
  }
}

/** 成功标记、提示和定位到的消息保持标记的时长。减少动画时同样保留。 */
export const FEEDBACK_MS = 2500;

/** 是否不播放动画：开了「减少动画」设置，或者系统设置了 `prefers-reduced-motion`。 */
export function prefersReducedMotion(setting: boolean): boolean {
  return setting || Boolean(window.matchMedia?.('(prefers-reduced-motion: reduce)').matches);
}

/** 以毫秒为单位读取时长令牌（如 `--duration-exit`）；没有这个令牌时为 0。 */
export function durationMs(token: string, element: Element = document.documentElement): number {
  const value = getComputedStyle(element).getPropertyValue(token).trim().split(/\s+/)[0] ?? '';
  const amount = Number.parseFloat(value);
  if (!Number.isFinite(amount)) return 0;
  return value.endsWith('ms') ? amount : value.endsWith('s') ? amount * 1000 : 0;
}

/** `usePresence` 为时有时无的值保存的状态：最后显示的值，以及它是否正在离开。 */
export interface Presence<T> {
  value: T;
  /** 退出动画播放期间为 true；元素在 `--duration-exit` 之后移除。 */
  closing: boolean;
}

/**
 * 值被清空后仍在屏幕上保留到退出动画播放完，让浮层、对话框和提示像进入时一样平滑地离开。
 * 退出结束后返回 null。
 */
export function usePresence<T>(value: T | null | undefined | false): Presence<T> | null {
  const [shown, setShown] = useState<T | null>(value || null);
  const [closing, setClosing] = useState(false);
  if (value) {
    if (value !== shown || closing) { setShown(value); setClosing(false); }
  } else if (shown !== null && !closing) {
    setClosing(true);
  }
  useEffect(() => {
    if (!closing) return;
    const timer = window.setTimeout(() => { setShown(null); setClosing(false); }, durationMs('--duration-exit'));
    return () => window.clearTimeout(timer);
  }, [closing]);
  return shown === null ? null : { value: shown, closing };
}
