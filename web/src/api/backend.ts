// Whether `weflow serve` is there and speaks the API this build was written for. `GET /health/events` needs no
// token; its `hello` is `/health`'s `{"status": "ok", "apiVersion": "2", "version": "5.0.0", "data": "…", "lang": "zh"}`
// (docs/serve-api.md, "Versions"). A server without it (5.0.0-rc.5 and older) is not supported.
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiUrl, checkToken, onAuth, token } from './client';
import { emitLive } from './live';
import { setLang, t } from '../i18n';

export interface Health {
  status: string;
  apiVersion: string;
  version: string;
  /** Changes with each start of the server and with its configuration (so with the account); see `scope.ts`. */
  data?: string;
  /** What may run at once (`--max-tasks`, `--max-reads`, `--max-media`). */
  limits?: { maxTasks?: number; maxReads?: number; maxMedia?: number };
  /** Whether the insight engine runs beside (`--insight`). */
  insight?: boolean;
  /** The language of the server's answers (#439), which the page shows itself in; older servers do not say. */
  lang?: 'zh' | 'en';
}

export type Backend =
  | { state: 'checking' }
  | { state: 'offline' }
  | { state: 'ready'; health: Health }
  | { state: 'incompatible'; health: Health; reason: string; advice: string }
  /** The server is there and suits this page, but refuses the token of this tab (#214). */
  | { state: 'unauthorized'; health: Health };

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
    return t('网页需要 API 版本 {want}，weflow 提供的是 {have}', { want: web.apiVersion, have: health.apiVersion });
  }
  const order = compareVersions(health.version, web.minBackendVersion);
  if (Number.isNaN(order) || order < 0) {
    return t('网页需要 weflow {want} 或更新的版本，当前是 {have}', { want: web.minBackendVersion, have: health.version });
  }
  return null;
}

/**
 * What to do about a server that does not suit this page (#214): a weflow too old (or of an older API) is upgraded,
 * or another WebUI picked that suits it; a WebUI of an older API than the server's is updated.
 */
export function adviceFor(health: Health, web = WEB): string {
  const ours = Number(web.apiVersion);
  const theirs = Number(health.apiVersion);
  if (health.apiVersion !== web.apiVersion && Number.isFinite(ours) && Number.isFinite(theirs) && theirs > ours) {
    return t('这个网页比 weflow 旧：在终端运行 weflow web update，换成适用的最新网页版本，再重新打开。');
  }
  return t('请把 weflow 升级到 {want} 或更新的版本；或运行 weflow web list --remote 找一个适用于当前 weflow 的网页版本，再用 weflow web use <版本> 换上。', { want: web.minBackendVersion });
}

function stateOf(health: Health): Backend {
  const reason = incompatibility(health);
  return reason ? { state: 'incompatible', health, reason, advice: adviceFor(health) } : { state: 'ready', health };
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
  /** Whether the server takes the token, asked when the live stream (which needs it) fails; see `checkToken`. */
  checkToken?: () => Promise<'ok' | 'unauthorized' | 'offline'>;
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
  /** Live messages (#201): the stream is `/api/push/messages`, which says `hello` and beats the same. */
  private live = false;
  /** The id of the last message event, to go on from it after a break (`last-event-id`). */
  private lastEventId = '';
  /** Whether the next `hello` is the page's very first, which needs no catching up. */
  private first = true;
  /**
   * The server refused the token (#214): the state stays `unauthorized` and the stream is `/health/events`, which
   * needs none, until another token is set.
   */
  private unauthorized = false;
  private stopAuth: (() => void) | null = null;
  private readonly open: (url: string) => Stream;
  private readonly loopback: boolean;
  private readonly random: () => number;
  private readonly checkToken: () => Promise<'ok' | 'unauthorized' | 'offline'>;

  constructor(private readonly onChange: (backend: Backend) => void, options: ConnectionOptions = {}) {
    this.open = options.open ?? (url => new EventSource(url));
    this.loopback = options.loopback ?? isLoopback(window.location.hostname);
    this.random = options.random ?? Math.random;
    this.checkToken = options.checkToken ?? (() => checkToken());
  }

  start(): void {
    this.stopped = false;
    window.addEventListener('online', this.online);
    window.addEventListener('offline', this.offline);
    document.addEventListener('visibilitychange', this.visible);
    this.stopAuth = onAuth(this.auth);
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    window.removeEventListener('online', this.online);
    window.removeEventListener('offline', this.offline);
    document.removeEventListener('visibilitychange', this.visible);
    this.stopAuth?.();
    this.stopAuth = null;
    this.drop();
  }

  /** Connects now, from the first wait again (the settings' 「重新检测」, the network back). */
  reconnect(): void {
    this.attempt = 0;
    this.connect();
  }

  /** Listens to the live messages (one stream for both, #221 a) or to the server's state alone. */
  setLive(live: boolean): void {
    if (live === this.live) return;
    this.live = live;
    if (!this.stopped && this.stream) this.reconnect();
  }

  private url(): string {
    if (!this.live || this.unauthorized) return EVENTS_URL;
    // EventSource sends no header: the token goes in the address, as for media
    return apiUrl('api/push/messages', { access_token: token() ?? undefined, 'last-event-id': this.lastEventId || undefined });
  }

  /** The state for what the server says, as long as it takes the token. */
  private judged(health: Health): Backend {
    const state = stateOf(health);
    return state.state === 'ready' && this.unauthorized ? { state: 'unauthorized', health } : state;
  }

  // A 401 from any request, or another token set by the person
  private readonly auth = (event: 'unauthorized' | 'token') => {
    if (event === 'token') {
      this.unauthorized = false;
      if (!this.stopped) this.reconnect();
      return;
    }
    if (!this.unauthorized) this.refused();
  };

  private refused(): void {
    this.unauthorized = true;
    const now = this.state;
    if (now.state === 'ready') this.set({ state: 'unauthorized', health: now.health });
    // the live stream needs the token: the state alone, until another one is set
    if (this.live && !this.stopped) this.reconnect();
  }

  private set(next: Backend): void {
    const now = this.state;
    const same = now.state === next.state && (next.state === 'checking' || next.state === 'offline'
      || ('health' in now && 'health' in next && JSON.stringify(now.health) === JSON.stringify(next.health)));
    if (same) return;
    this.state = next;
    // the page speaks the server's language (#439)
    if ('health' in next) setLang(next.health.lang);
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
    const stream = this.open(this.url());
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
        this.set(this.judged(JSON.parse(event.data) as Health));
      } catch {
        this.lost();
        return;
      }
      // back after a break, or live just turned on: messages may have come meanwhile
      if (this.live && !this.first) emitLive({ type: 'resumed' });
      this.first = false;
    });
    for (const type of ['message.new', 'message.revoke'] as const) {
      stream.addEventListener(type, event => {
        if (this.stream !== stream) return;
        heard();
        if (event.lastEventId) this.lastEventId = event.lastEventId;
        try {
          const { sessionId } = JSON.parse(event.data) as { sessionId?: string };
          if (sessionId) emitLive({ type, sessionId });
        } catch {
          // an event without its conversation tells nothing to show
        }
      });
    }
    stream.addEventListener('heartbeat', event => {
      if (this.stream !== stream) return;
      heard();
      // the data changed under an open stream: another account (see scope.ts)
      const now = this.state;
      if (!('health' in now)) return;
      try {
        const { data } = JSON.parse(event.data) as { data?: string };
        if (data !== undefined && data !== now.health.data) this.set(this.judged({ ...now.health, data }));
      } catch {
        // a heartbeat says the stream is alive whatever it holds
      }
    });
    // the server stopped on purpose (Ctrl+C): offline at once, as when it is lost, and back once it starts again
    stream.addEventListener('bye', () => {
      if (this.stream === stream) this.lost();
    });
    stream.onerror = () => {
      if (this.stream !== stream) return;
      this.lost();
      if (!this.live || this.unauthorized) return;
      // The live stream needs the token, and an EventSource cannot tell a refused one (401) from a lost server: ask.
      // Refused, it turns to the state alone at once, which then says `unauthorized`.
      this.checkToken().then(answer => {
        if (answer === 'unauthorized' && !this.stopped && !this.unauthorized) this.refused();
      }, () => { /* no answer: lost, as said */ });
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
    if (!('health' in this.state)) this.reconnect();
    else if (Date.now() - this.lastEvent > WATCHDOG_MS) this.reconnect();
  };
}

/**
 * The backend's state, from one event stream (see `Connection`); `recheck` connects again at once. With `live` the
 * stream is the live messages' (see live.ts).
 */
export function useBackend(live = false): [Backend, () => void] {
  const [backend, setBackend] = useState<Backend>({ state: 'checking' });
  const connection = useRef<Connection | null>(null);
  const recheck = useCallback(() => connection.current?.reconnect(), []);
  const wanted = useRef(live);
  wanted.current = live;

  useEffect(() => {
    const kept = new Connection(setBackend);
    kept.setLive(wanted.current);
    connection.current = kept;
    kept.start();
    return () => {
      kept.stop();
      connection.current = null;
    };
  }, []);

  useEffect(() => { connection.current?.setLive(live); }, [live]);

  return [backend, recheck];
}
