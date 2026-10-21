// What the tests import as '#test' (#361): bun:test, and a `vi` with what they use of Vitest's that Bun's lacks
// (stubGlobal, unstubAllGlobals, setSystemTime, advanceTimersByTimeAsync, waitFor, fake timers for Date alone).
import { setSystemTime as setTime, vi as bunVi } from 'bun:test';

export { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, onTestFinished, test } from 'bun:test';

const realSetTimeout = globalThis.setTimeout;
const fake = { useFakeTimers: bunVi.useFakeTimers, useRealTimers: bunVi.useRealTimers };
const stubbed = new Map<string, PropertyDescriptor | undefined>();
let dateOnly = false;

/** The promise callbacks queued so far, and those they queue in turn. */
async function settle() {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

export const vi = Object.assign(bunVi, {
  stubGlobal(name: string, value: unknown) {
    if (!stubbed.has(name)) stubbed.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true, enumerable: true });
    return vi;
  },
  unstubAllGlobals() {
    for (const [name, original] of stubbed) {
      if (original) Object.defineProperty(globalThis, name, original);
      else delete (globalThis as Record<string, unknown>)[name];
    }
    stubbed.clear();
    return vi;
  },
  /** `{ toFake: ['Date'] }` fakes the clock alone, so that timers stay real; anything else, all of them */
  useFakeTimers(options?: { toFake?: string[]; now?: number | Date }) {
    dateOnly = !!options?.toFake?.length && options.toFake.every(name => name === 'Date');
    if (dateOnly) setTime(options?.now ?? new Date());
    else fake.useFakeTimers.call(bunVi, options?.now === undefined ? undefined : { now: options.now });
    return vi;
  },
  useRealTimers() {
    if (dateOnly) setTime();
    else fake.useRealTimers.call(bunVi);
    dateOnly = false;
    return vi;
  },
  setSystemTime(now: number | Date) {
    setTime(now);
    return vi;
  },
  /** As Vitest's: the timers due within `ms` fire in turn, the promise callbacks they queue settling in between */
  async advanceTimersByTimeAsync(ms: number) {
    await settle();
    if (ms === 0) bunVi.advanceTimersByTime(0);
    for (let i = 0; i < ms; i++) {
      bunVi.advanceTimersByTime(1);
      await settle();
    }
    await settle();
    return vi;
  },
  /** `callback` once it stops throwing (or rejecting), trying every `interval` ms for `timeout` ms */
  async waitFor<T>(callback: () => T | Promise<T>, options: { timeout?: number; interval?: number } = {}): Promise<T> {
    const { timeout = 1000, interval = 50 } = options;
    const end = performance.now() + timeout;
    for (;;) {
      try {
        return await callback();
      } catch (error) {
        if (performance.now() >= end) throw error;
        if (!dateOnly && bunVi.isFakeTimers()) bunVi.advanceTimersByTime(interval);
        await new Promise(resolve => realSetTimeout(resolve, interval));
      }
    }
  },
});
