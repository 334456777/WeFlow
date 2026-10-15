// Motion helpers (#193). Durations and curves live in styles/tokens.css; scripts read them from there so the
// 「减少动画」 switch and `prefers-reduced-motion` (both set the durations to 0ms) apply to scripts too.
import { useEffect, useState } from 'react';

/** Spring presets for when Motion for React comes in; CSS approximates them with the curves in tokens.css. */
export const springs = {
  /** Buttons and switches. */
  control: { stiffness: 600, damping: 30, mass: 1 },
  /** Popovers and cards opening. */
  overlay: { stiffness: 320, damping: 28, mass: 1 },
  /** Success, copied and other light feedback. */
  feedback: { stiffness: 380, damping: 18, mass: 1 },
  /** Dragged panels settling, without overshoot. */
  settle: { stiffness: 1200, damping: 80, mass: 0.1 },
} as const;

/** How long a success mark, a notice or a located message stays marked. Kept with reduced motion. */
export const FEEDBACK_MS = 2500;

/** Reads a duration token such as `--duration-exit` in milliseconds; 0 when it is missing. */
export function durationMs(token: string, element: Element = document.documentElement): number {
  const value = getComputedStyle(element).getPropertyValue(token).trim();
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
