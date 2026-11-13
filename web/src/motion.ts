// Motion helpers (#193). Durations and curves live in styles/tokens.css; scripts read them from there so the
// 「减少动画」 switch and `prefers-reduced-motion` (both set the durations to 0ms) apply to scripts too.
import { useEffect, useState } from 'react';

/**
 * A duration token and a curve token (`--duration-panel`, `--ease-snap`) as the Web Animations API takes them; a
 * duration of 0 with reduced motion or without the token, and `ease` without the curve.
 */
export function timing(duration: string, easing: string, element: Element = document.documentElement): { duration: number; easing: string } {
  return { duration: durationMs(duration, element), easing: getComputedStyle(element).getPropertyValue(easing).trim() || 'ease' };
}

/**
 * Brings content in as `content-in` in app.css does (#776): from transparent and half `--enter-offset` lower, over
 * `--duration-base` with `--ease-out`. For content that is not mounted anew, such as another page in the same panel or
 * another item's detail in the same pane; nothing with reduced motion, where the tokens are 0.
 */
export function contentIn(elements: Iterable<Element>, root: Element = document.documentElement): void {
  const options = timing('--duration-base', '--ease-out', root);
  if (!options.duration) return;
  const offset = getComputedStyle(root).getPropertyValue('--enter-offset').trim() || '0px';
  const frames = [{ opacity: 0, transform: `translateY(calc(.5 * ${offset}))` }, { opacity: 1, transform: 'none' }];
  for (const element of elements) element.animate?.(frames, options);
}

/** How long a success mark, a notice or a located message stays marked. Kept with reduced motion. */
export const FEEDBACK_MS = 2500;

/** Whether motion is to be left out: the 「减少动画」 setting, or the system's `prefers-reduced-motion`. */
export function prefersReducedMotion(setting: boolean): boolean {
  return setting || Boolean(window.matchMedia?.('(prefers-reduced-motion: reduce)').matches);
}

/** Reads a duration token such as `--duration-exit` in milliseconds; 0 when it is missing. */
export function durationMs(token: string, element: Element = document.documentElement): number {
  const value = getComputedStyle(element).getPropertyValue(token).trim().split(/\s+/)[0] ?? '';
  const amount = Number.parseFloat(value);
  if (!Number.isFinite(amount)) return 0;
  return value.endsWith('ms') ? amount : value.endsWith('s') ? amount * 1000 : 0;
}

export interface Presence<T> {
  value: T;
  /** True while the exit animation plays; the element is removed after `--duration-exit`. */
  closing: boolean;
}

/**
 * Keeps a value on screen for the exit animation after it is cleared, so popovers, dialogs and notices can
 * leave as smoothly as they come. Returns null once the exit is over.
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
