// Motion helpers (#193). Durations and curves live in styles/tokens.css; scripts read them from there so the
// 「减少动画」 switch and `prefers-reduced-motion` (both set the durations to 0ms) apply to scripts too.
import { useEffect, useState } from 'react';
import { spring } from 'motion';

/**
 * The springs of the interface (#652), the one place their parameters are written: Motion traces each as a CSS
 * `linear()` curve, which {@link installSprings} writes into the `--spring-<name>` tokens the stylesheets animate with.
 */
export const springs = {
  /** Buttons and switches: a thumb, a knob, a check, the hover hollow; a little overshoot. */
  control: { stiffness: 600, damping: 30, mass: 1 },
  /** Popovers, dialogs and cards opening. */
  overlay: { stiffness: 320, damping: 28, mass: 1 },
  /** A notice and other light feedback: the most overshoot. */
  feedback: { stiffness: 380, damping: 18, mass: 1 },
  /** Panels settling: a dragged card, the sidebar folding; no overshoot. */
  settle: { stiffness: 1200, damping: 80, mass: 0.1 },
} as const;

export type SpringName = keyof typeof springs;

/** A spring as CSS, `<duration>ms linear(…)`: as long as it takes to come to rest, and the curve it traces. */
export function springCss(name: SpringName): string {
  return spring({ keyframes: [0, 1], ...springs[name] }).toString();
}

/**
 * Writes every spring as its `--spring-<name>` token, and its duration alone as `--spring-<name>-duration` (for a
 * delay that waits for it), in a stylesheet put before all the others, so that the reduced-motion rules of
 * tokens.css, later in the cascade, still set the tokens to 0ms. A browser without `linear()` easing gets each
 * spring's duration on an ease-out curve.
 */
export function installSprings(doc: Document = document): void {
  const linear = typeof CSS !== 'undefined' && CSS.supports?.('transition-timing-function', 'linear(0, 1)');
  const tokens = (Object.keys(springs) as SpringName[]).map(name => {
    const css = springCss(name);
    const duration = css.slice(0, css.indexOf(' '));
    return '--spring-' + name + ': ' + (linear ? css : duration + ' ease-out') + '; --spring-' + name + '-duration: ' + duration + ';';
  });
  const style = doc.getElementById('weflow-springs') ?? doc.createElement('style');
  style.id = 'weflow-springs';
  style.textContent = ':root { ' + tokens.join(' ') + ' }';
  doc.head.prepend(style);
}

/** A spring token as the Web Animations API takes it; a duration of 0 with reduced motion or without the token. */
export function springTiming(name: SpringName, element: Element = document.documentElement): { duration: number; easing: string } {
  const value = getComputedStyle(element).getPropertyValue('--spring-' + name).trim();
  const space = value.indexOf(' ');
  return { duration: durationMs('--spring-' + name, element), easing: space < 0 ? 'linear' : value.slice(space + 1) };
}

/**
 * Brings content in as `content-in` in app.css does (#776): from transparent and half `--enter-offset` lower, over
 * `--duration-base` with `--ease-out`. For content that is not mounted anew, such as another page in the same panel or
 * another item's detail in the same pane; nothing with reduced motion, where the tokens are 0.
 */
export function contentIn(elements: Iterable<Element>, root: Element = document.documentElement): void {
  const duration = durationMs('--duration-base', root);
  if (!duration) return;
  const style = getComputedStyle(root);
  const offset = style.getPropertyValue('--enter-offset').trim() || '0px';
  const easing = style.getPropertyValue('--ease-out').trim() || 'ease-out';
  const frames = [{ opacity: 0, transform: `translateY(calc(.5 * ${offset}))` }, { opacity: 1, transform: 'none' }];
  for (const element of elements) element.animate?.(frames, { duration, easing });
}

/** How long a success mark, a notice or a located message stays marked. Kept with reduced motion. */
export const FEEDBACK_MS = 2500;

/** Whether motion is to be left out: the 「减少动画」 setting, or the system's `prefers-reduced-motion`. */
export function prefersReducedMotion(setting: boolean): boolean {
  return setting || Boolean(window.matchMedia?.('(prefers-reduced-motion: reduce)').matches);
}

/**
 * Reads a duration token such as `--duration-exit` in milliseconds, or the duration a `--spring-<name>` token starts
 * with; 0 when it is missing.
 */
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
