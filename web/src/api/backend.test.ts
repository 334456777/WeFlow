import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { compareVersions, Connection, incompatibility, retryDelay, WATCHDOG_MS, type Backend, type Stream } from './backend';
import { onLive, type LiveEvent } from './live';

const web = { name: 'weflow-web', version: '0.1.0', apiVersion: '1', minBackendVersion: '5.0.0-rc.4' };

afterEach(() => vi.unstubAllGlobals());

describe('compareVersions', () => {
  it('orders versions as SemVer does', () => {
    const sorted = ['4.2.1', '5.0.0-rc.3', '5.0.0-rc.4', '5.0.0-rc.4-3-gabc1234', '5.0.0-rc.10', '5.0.0', '5.0.1', '5.1.0'];
    for (let i = 1; i < sorted.length; i++) expect(compareVersions(sorted[i - 1], sorted[i]), sorted[i - 1] + ' < ' + sorted[i]).toBeLessThan(0);
    expect(compareVersions('v5.0.0', '5.0.0')).toBe(0);
    expect(compareVersions('5.0.0-alpha', '5.0.0-alpha.1')).toBeLessThan(0);
    expect(compareVersions('5.0.0-1', '5.0.0-alpha')).toBeLessThan(0);
  });

  it('gives NaN for something that is not a version', () => {
    expect(compareVersions('unknown', '5.0.0')).toBeNaN();
  });
});

describe('incompatibility', () => {
  it('accepts the same API on a new enough backend', () => {
    expect(incompatibility({ status: 'ok', apiVersion: '1', version: '5.0.0-rc.4' }, web)).toBeNull();
    expect(incompatibility({ status: 'ok', apiVersion: '1', version: '5.0.0-rc.4-12-g54c22b9' }, web)).toBeNull();
    expect(incompatibility({ status: 'ok', apiVersion: '1', version: '5.2.0' }, web)).toBeNull();
  });

  it('refuses another API or an older backend', () => {
    expect(incompatibility({ status: 'ok', apiVersion: '2', version: '6.0.0' }, web)).toContain('API');
    expect(incompatibility({ status: 'ok', apiVersion: '1', version: '5.0.0-rc.3' }, web)).toContain('5.0.0-rc.4');
    expect(incompatibility({ status: 'ok', apiVersion: '1', version: 'unknown' }, web)).not.toBeNull();
  });
});

/** A stand-in for EventSource: the test says what the server sends. */
class FakeStream implements Stream {
  onerror: ((event: Event) => void) | null = null;
  closed = false;
  private listeners = new Map<string, ((event: MessageEvent) => void)[]>();
  constructor(readonly url: string) {}
  addEventListener(type: string, listener: (event: MessageEvent) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  close() { this.closed = true; }
  send(type: string, data: unknown) {
    for (const listener of this.listeners.get(type) ?? []) listener(new MessageEvent(type, { data: JSON.stringify(data) }));
  }
  fail() { this.onerror?.(new Event('error')); }
  emitWithId(type: string, data: unknown, lastEventId: string) {
    for (const listener of this.listeners.get(type) ?? []) listener(new MessageEvent(type, { data: JSON.stringify(data), lastEventId }));
  }
}

const health = { status: 'ok', apiVersion: '1', version: '9.0.0', data: 'a' };

/** A connection on fake streams and timers: the streams it opened, and the states it reported. */
function connection(options: { loopback?: boolean } = {}) {
  const streams: FakeStream[] = [];
  const states: Backend[] = [];
  const live = new Connection(state => states.push(state), {
    open: url => { const stream = new FakeStream(url); streams.push(stream); return stream; },
    loopback: options.loopback ?? true,
    random: () => 0.5,
  });
  live.start();
  const last = () => streams[streams.length - 1];
  return { live, streams, states, last, now: () => states[states.length - 1] };
}

describe('Connection (#221)', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it('is ready on hello, and offline the moment the stream breaks', () => {
    const c = connection();
    expect(c.streams).toHaveLength(1);
    expect(c.last().url).toBe('/health/events');
    c.last().send('hello', health);
    expect(c.now()).toEqual({ state: 'ready', health });
    // heartbeats change nothing
    c.last().send('heartbeat', { data: 'a' });
    expect(c.states).toHaveLength(1);
    c.last().fail();
    expect(c.now()).toEqual({ state: 'offline' });
    expect(c.streams[0].closed).toBe(true);
    c.live.stop();
  });

  it('is offline when the server says bye, and comes back once it starts again', () => {
    const c = connection();
    c.last().send('hello', health);
    c.last().send('bye', { reason: 'stopped' });
    expect(c.now()).toEqual({ state: 'offline' });
    expect(c.streams[0].closed).toBe(true);
    vi.advanceTimersByTime(1_000);
    expect(c.streams).toHaveLength(2);
    c.last().send('hello', health);
    expect(c.now().state).toBe('ready');
    c.live.stop();
  });

  it('says when the server does not suit this page', () => {
    const c = connection();
    c.last().send('hello', { ...health, apiVersion: '2' });
    expect(c.now().state).toBe('incompatible');
    c.live.stop();
  });

  it('tells a new account by the data of a heartbeat', () => {
    const c = connection();
    c.last().send('hello', health);
    c.last().send('heartbeat', { data: 'b' });
    expect(c.now()).toEqual({ state: 'ready', health: { ...health, data: 'b' } });
    c.live.stop();
  });

  it('drops a silent stream after the watchdog and connects again', () => {
    const c = connection();
    c.last().send('hello', health);
    vi.advanceTimersByTime(15_000);
    c.last().send('heartbeat', { data: 'a' });
    vi.advanceTimersByTime(WATCHDOG_MS - 1);
    expect(c.now().state).toBe('ready');
    vi.advanceTimersByTime(1);
    expect(c.now()).toEqual({ state: 'offline' });
    expect(c.streams[0].closed).toBe(true);
    vi.advanceTimersByTime(1_000);
    expect(c.streams).toHaveLength(2);
    c.live.stop();
  });

  it('waits 1, 2, 4 … 30 s between tries, and starts over once connected', () => {
    expect([0, 1, 2, 3, 4, 5, 6, 9].map(n => retryDelay(n, () => 0.5))).toEqual([1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000, 30_000]);
    expect(retryDelay(0, () => 0)).toBe(800);
    expect(retryDelay(9, () => 1)).toBe(30_000);
    const c = connection();
    const waits: number[] = [];
    for (let i = 0; i < 7; i++) {
      c.last().fail();
      const before = c.streams.length;
      let waited = 0;
      while (c.streams.length === before) { vi.advanceTimersByTime(100); waited += 100; }
      waits.push(waited);
    }
    expect(waits).toEqual([1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000]);
    c.last().send('hello', health);
    c.last().fail();
    vi.advanceTimersByTime(1_000);
    expect(c.streams).toHaveLength(9);
    c.live.stop();
  });

  it('connects at once when the network or the tab comes back, or when asked', () => {
    const c = connection();
    c.last().fail();
    for (let i = 0; i < 3; i++) { vi.advanceTimersByTime(30_000); c.last().fail(); }
    const opened = c.streams.length;
    window.dispatchEvent(new Event('online'));
    expect(c.streams).toHaveLength(opened + 1);
    c.last().fail();
    document.dispatchEvent(new Event('visibilitychange'));
    expect(c.streams).toHaveLength(opened + 2);
    c.last().fail();
    c.live.reconnect();
    expect(c.streams).toHaveLength(opened + 3);
    // ready and heard from lately: coming back changes nothing
    c.last().send('hello', health);
    document.dispatchEvent(new Event('visibilitychange'));
    expect(c.streams).toHaveLength(opened + 3);
    // ready, but the timers slept in the background past the watchdog
    vi.setSystemTime(Date.now() + WATCHDOG_MS + 1);
    document.dispatchEvent(new Event('visibilitychange'));
    expect(c.streams).toHaveLength(opened + 4);
    c.live.stop();
  });

  it('does not turn red when the network goes on this machine, but does across machines', () => {
    const near = connection({ loopback: true });
    near.last().send('hello', health);
    window.dispatchEvent(new Event('offline'));
    expect(near.now().state).toBe('ready');
    // it only checked again
    expect(near.streams).toHaveLength(2);
    near.live.stop();

    const far = connection({ loopback: false });
    far.last().send('hello', health);
    window.dispatchEvent(new Event('offline'));
    expect(far.now()).toEqual({ state: 'offline' });
    far.live.stop();
  });

  it('listens to the live messages on one stream, and goes on from the last one after a break (#201)', () => {
    sessionStorage.setItem('weflow.token', 't+1');
    const heard: LiveEvent[] = [];
    const off = onLive(event => heard.push(event));
    const c = connection();
    c.last().send('hello', health);
    expect(heard).toEqual([]);
    // turned on later: what came while it was off is read too
    c.live.setLive(true);
    expect(c.last().url).toBe('/api/push/messages?access_token=t%2B1');
    c.last().send('hello', health);
    expect(heard).toEqual([{ type: 'resumed' }]);
    c.last().send('ready', { success: true });
    for (const listener of ['message.new', 'message.revoke']) {
      c.last().emitWithId(listener, { sessionId: 'wxid_a', content: 'hi' }, listener === 'message.new' ? '1760000000123' : '1760000000124');
    }
    expect(heard.slice(1)).toEqual([{ type: 'message.new', sessionId: 'wxid_a' }, { type: 'message.revoke', sessionId: 'wxid_a' }]);
    // a break: back from the last event, and anything may have come meanwhile
    c.last().fail();
    vi.advanceTimersByTime(1_000);
    expect(c.last().url).toBe('/api/push/messages?access_token=t%2B1&last-event-id=1760000000124');
    c.last().send('hello', health);
    expect(heard.at(-1)).toEqual({ type: 'resumed' });
    // turned off: the state alone again
    c.live.setLive(false);
    expect(c.last().url).toBe('/health/events');
    off();
    c.live.stop();
    sessionStorage.clear();
  });

  it('needs no catching up on the first hello of a page that starts live', () => {
    const heard: LiveEvent[] = [];
    const off = onLive(event => heard.push(event));
    const streams: FakeStream[] = [];
    const live = new Connection(() => {}, { open: url => { const s = new FakeStream(url); streams.push(s); return s; }, loopback: true });
    live.setLive(true);
    live.start();
    expect(streams).toHaveLength(1);
    streams[0].send('hello', health);
    expect(heard).toEqual([]);
    off();
    live.stop();
  });

  it('stops for good', () => {
    const c = connection();
    c.live.stop();
    expect(c.last().closed).toBe(true);
    vi.advanceTimersByTime(60_000);
    window.dispatchEvent(new Event('online'));
    expect(c.streams).toHaveLength(1);
  });
});
