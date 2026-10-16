// Whether `weflow serve` is there and speaks the API this build was written for. `GET /health/events` needs no
// token; its `hello` is `/health`'s `{"status": "ok", "apiVersion": "1", "version": "5.0.0", "data": "…"}`
// (docs/serve-api.md, "Versions"). A server without it (5.0.0-rc.5 and older) is not supported.
import { useCallback, useEffect, useRef, useState } from 'react';

export interface Health {
  status: string;
  apiVersion: string;
  version: string;
  /** Changes with each start of the server and with its configuration (so with the account); see `scope.ts`. */
  data?: string;
}

export type Backend =
  | { state: 'checking' }
  | { state: 'offline' }
  | { state: 'ready'; health: Health }
  | { state: 'incompatible'; health: Health; reason: string };

export const WEB = __WEFLOW_WEB__;

type Identifier = number | string;

function parse(version: string): { core: number[]; pre: Identifier[] } | null {
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+.*)?$/.exec(version.trim());
  if (!match) return null;
  const pre = match[4] ? match[4].split('.').map(p => (/^\d+$/.test(p) ? Number(p) : p)) : [];
  return { core: [Number(match[1]), Number(match[2]), Number(match[3])], pre };
}

/** A development build reports `git describe`: the last tag, then the commits since it (`5.0.0-rc.4-3-gabc1234`). */
function described(version: string): { tag: string; ahead: number } {
  const match = /^(.+)-(\d+)-g[0-9a-f]+$/.exec(version.trim());
  return match ? { tag: match[1], ahead: Number(match[2]) } : { tag: version, ahead: 0 };
}

/**
 * Orders two versions as SemVer does: a pre-release comes before its release, numeric identifiers before
 * others. A `git describe` version sorts right after its tag and before the next one. Returns NaN when either is
 * not a version.
 */
export function compareVersions(a: string, b: string): number {
  const p = described(a);
  const q = described(b);
  const order = compareTags(p.tag, q.tag);
  return order === 0 ? p.ahead - q.ahead : order;
}

function compareTags(a: string, b: string): number {
  const x = parse(a);
  const y = parse(b);
  if (!x || !y) return NaN;
  for (let i = 0; i < 3; i++) if (x.core[i] !== y.core[i]) return x.core[i] - y.core[i];
  if (!x.pre.length || !y.pre.length) return y.pre.length - x.pre.length;
  for (let i = 0; i < Math.min(x.pre.length, y.pre.length); i++) {
    const p = x.pre[i];
    const q = y.pre[i];
    if (p === q) continue;
    if (typeof p === 'number' && typeof q === 'number') return p - q;
    if (typeof p === 'number') return -1;
    if (typeof q === 'number') return 1;
    return p < q ? -1 : 1;
  }
  return x.pre.length - y.pre.length;
}

/** Why this build cannot use the server, or null when it can. */
export function incompatibility(health: Health, web = WEB): string | null {
  if (health.apiVersion !== web.apiVersion) {
    return '网页需要 API 版本 ' + web.apiVersion + '，weflow 提供的是 ' + health.apiVersion;
  }
  const order = compareVersions(health.version, web.minBackendVersion);
  if (Number.isNaN(order) || order < 0) {
    return '网页需要 weflow ' + web.minBackendVersion + ' 或更新的版本，当前是 ' + health.version;
  }
  return null;
}

function stateOf(health: Health): Backend {
  const reason = incompatibility(health);
  return reason ? { state: 'incompatible', health, reason } : { state: 'ready', health };
}

/** The server's event stream (#221): `hello` with what `/health` says, then a `heartbeat` every 15 s. */
export const EVENTS_URL = '/health/events';
/** Without any event for this long (two and a half heartbeats) the stream counts as dead. */
export const WATCHDOG_MS = 37_500;
/** Waits before connecting again: 1 s, doubled each time, at most 30 s. */
const FIRST_RETRY_MS = 1_000;
const LAST_RETRY_MS = 30_000;

/** What the connection needs from `EventSource`, so a test can hand it another. */
export interface Stream {
  addEventListener(type: string, listener: (event: MessageEvent) => void): void;
  onerror: ((event: Event) => void) | null;
  close(): void;
}

export interface ConnectionOptions {
  open?: (url: string) => Stream;
  /** True when the page and the server are on this machine, where losing the network does not cut them apart. */
  loopback?: boolean;
  random?: () => number;
}

/** The wait before attempt `n` (0 first): doubling from 1 s up to 30 s, give or take a fifth so tabs spread out. */
export function retryDelay(attempt: number, random = Math.random): number {
  const base = Math.min(LAST_RETRY_MS, FIRST_RETRY_MS * 2 ** attempt);
  return Math.min(LAST_RETRY_MS, Math.round(base * (0.8 + 0.4 * random())));
}

const isLoopback = (host: string) => host === 'localhost' || host === '[::1]' || host === '::1' || /^127\./.test(host);

/**
 * One long-lived stream to the server instead of asking it again and again (#221). The state turns `offline` the
 * moment the stream breaks (the server stopped or crashed); a stream that stays silent past the watchdog (a
 * half-open connection: Wi-Fi gone, a NAT that forgot it) is dropped too. It connects again after 1, 2, 4 … 30 s,
 * at once when the network or the tab comes back, and on `reconnect()`. It reconnects itself rather than letting
 * `EventSource` do it, as the browser's own retry is not ours to pace and stops for good on some failures.
 */
export class Connection {
  private state: Backend = { state: 'checking' };
  private stream: Stream | null = null;
  private attempt = 0;
  private retry: ReturnType<typeof setTimeout> | undefined;
  private watchdog: ReturnType<typeof setTimeout> | undefined;
  private lastEvent = 0;
  private stopped = false;
  private readonly open: (url: string) => Stream;
  private readonly loopback: boolean;
  private readonly random: () => number;

  constructor(private readonly onChange: (backend: Backend) => void, options: ConnectionOptions = {}) {
    this.open = options.open ?? (url => new EventSource(url));
    this.loopback = options.loopback ?? isLoopback(window.location.hostname);
    this.random = options.random ?? Math.random;
  }

  start(): void {
    this.stopped = false;
    window.addEventListener('online', this.online);
    window.addEventListener('offline', this.offline);
    document.addEventListener('visibilitychange', this.visible);
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    window.removeEventListener('online', this.online);
    window.removeEventListener('offline', this.offline);
    document.removeEventListener('visibilitychange', this.visible);
    this.drop();
  }

  /** Connects now, from the first wait again (the settings' 「重新检测」, the network back). */
  reconnect(): void {
    this.attempt = 0;
    this.connect();
  }

  private set(next: Backend): void {
    const now = this.state;
    const same = now.state === next.state && (next.state === 'checking' || next.state === 'offline'
      || ('health' in now && 'health' in next && JSON.stringify(now.health) === JSON.stringify(next.health)));
    if (same) return;
    this.state = next;
    this.onChange(next);
  }

  private drop(): void {
    clearTimeout(this.retry);
    clearTimeout(this.watchdog);
    this.stream?.close();
    this.stream = null;
  }

  private connect(): void {
    if (this.stopped) return;
    this.drop();
    const stream = this.open(EVENTS_URL);
    this.stream = stream;
    const heard = () => {
      this.lastEvent = Date.now();
      clearTimeout(this.watchdog);
      this.watchdog = setTimeout(() => this.lost(), WATCHDOG_MS);
    };
    stream.addEventListener('hello', event => {
      if (this.stream !== stream) return;
      heard();
      this.attempt = 0;
      try {
        this.set(stateOf(JSON.parse(event.data) as Health));
      } catch {
        this.lost();
      }
    });
    stream.addEventListener('heartbeat', event => {
      if (this.stream !== stream) return;
      heard();
      // the data changed under an open stream: another account (see scope.ts)
      const now = this.state;
      if (!('health' in now)) return;
      try {
        const { data } = JSON.parse(event.data) as { data?: string };
        if (data !== undefined && data !== now.health.data) this.set(stateOf({ ...now.health, data }));
      } catch {
        // a heartbeat says the stream is alive whatever it holds
      }
    });
    // the server stopped on purpose (Ctrl+C): offline at once, as when it is lost, and back once it starts again
    stream.addEventListener('bye', () => {
      if (this.stream === stream) this.lost();
    });
    stream.onerror = () => {
      if (this.stream === stream) this.lost();
    };
    heard();
  }

  /** The stream broke or went silent: offline now, and another try after the wait. */
  private lost(): void {
    this.drop();
    this.set({ state: 'offline' });
    if (this.stopped) return;
    this.retry = setTimeout(() => this.connect(), retryDelay(this.attempt, this.random));
    this.attempt += 1;
  }

  private readonly online = () => this.reconnect();

  // On this machine the server is still there without a network: check at once, but do not turn red for it
  private readonly offline = () => {
    if (this.loopback) this.reconnect();
    else this.lost();
  };

  // A tab in the background has its timers slowed down: on coming back, connect at once unless all is well
  private readonly visible = () => {
    if (document.visibilityState !== 'visible') return;
    if (this.state.state !== 'ready' && this.state.state !== 'incompatible') this.reconnect();
    else if (Date.now() - this.lastEvent > WATCHDOG_MS) this.reconnect();
  };
}

/** The backend's state, from one event stream (see `Connection`); `recheck` connects again at once. */
export function useBackend(): [Backend, () => void] {
  const [backend, setBackend] = useState<Backend>({ state: 'checking' });
  const connection = useRef<Connection | null>(null);
  const recheck = useCallback(() => connection.current?.reconnect(), []);

  useEffect(() => {
    const live = new Connection(setBackend);
    connection.current = live;
    live.start();
    return () => {
      live.stop();
      connection.current = null;
    };
  }, []);

  return [backend, recheck];
}
