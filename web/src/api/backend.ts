// `weflow serve` 是否在线，以及它提供的是不是这个构建所用的 API。`GET /health/events` 不需要 token，
// 它的 `hello` 就是 `/health` 的回答（见 Wiki 的「命令 API」页）：
// `{"status": "ok", "apiVersion": "…", "version": "5.0.0", "data": "…", "lang": "zh"}`。
// 不支持没有这个路由的服务端（5.0.0-rc.5 及更早的版本）。
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiUrl, checkToken, onAuth, token } from './client';
import { emitLive } from './live';
import { onPresence, present } from './presence';
import { setLang, t } from '../i18n';

/** `/health` 的回答，也是 `/health/events` 的 `hello`。 */
export interface Health {
  status: string;
  apiVersion: string;
  version: string;
  /** 每次启动服务端和配置变化时都会变（所以账号变了它也会变）；见 `scope.ts`。 */
  data?: string;
  /** 能同时运行的数量（`--max-tasks`、`--max-reads`、`--max-media`）。 */
  limits?: { maxTasks?: number; maxReads?: number; maxMedia?: number };
  /** 是否同时运行见解引擎（`--insight`）。 */
  insight?: boolean;
  /** 服务端回答所用的语言（#439），网页也用这种语言显示；较早的服务端不给出。 */
  lang?: 'zh' | 'en';
  /** 服务端所在电脑的操作系统；较早的服务端不给出。 */
  platform?: 'windows' | 'macos' | 'linux';
}

/** 与服务端的连接状态，以及连接得知的服务端信息。 */
export type Backend =
  | { state: 'checking' }
  /** `held`：服务端断开前已经就绪，从它读到的记录继续显示，直到它回来。 */
  | { state: 'offline'; held?: true }
  | { state: 'ready'; health: Health }
  | { state: 'incompatible'; health: Health; reason: string; advice: string }
  /** 服务端在线、也适合这个网页，但拒绝了这个标签页的 token（#214）。 */
  | { state: 'unauthorized'; health: Health };

/** 这个构建的名称、版本和需要的 API，取自 `package.json`。 */
export const WEB = __WEFLOW_WEB__;

type Identifier = number | string;

function parse(version: string): { core: number[]; pre: Identifier[] } | null {
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+.*)?$/.exec(version.trim());
  if (!match) return null;
  const pre = match[4] ? match[4].split('.').map(p => (/^\d+$/.test(p) ? Number(p) : p)) : [];
  return { core: [Number(match[1]), Number(match[2]), Number(match[3])], pre };
}

/**
 * 拆开开发版报告的 `git describe` 版本：最近的 tag 和之后的提交数（`5.0.0-rc.4-3-gabc1234`）；
 * 不是这种形式时提交数为 0。
 */
function described(version: string): { tag: string; ahead: number } {
  const match = /^(.+)-(\d+)-g[0-9a-f]+$/.exec(version.trim());
  return match ? { tag: match[1], ahead: Number(match[2]) } : { tag: version, ahead: 0 };
}

/**
 * 按语义化版本（SemVer）比较两个版本：预发布版排在正式版之前，数字标识符排在其他标识符之前。
 * `git describe` 的版本排在它的 tag 之后、下一个版本之前。任一个不是版本号时返回 NaN。
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

/** 这个构建不能使用这个服务端的原因；能用时为 null。 */
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
 * 服务端不适合这个网页时的处理办法（#214）：weflow 太旧（或 API 版本较旧）时升级 weflow，或者换一个适用于它的
 * WebUI；WebUI 的 API 版本比服务端旧时更新 WebUI。
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

/** 服务端的事件流（#221）：先是内容与 `/health` 相同的 `hello`，之后每 15 秒一个 `heartbeat`。 */
export const EVENTS_URL = '/health/events';
/** 这么长时间（两个半心跳）收不到任何事件，就认为事件流已经断了。 */
export const WATCHDOG_MS = 37_500;
/** 重新连接之前的等待：从 1 秒开始，每次加倍，最多 30 秒。 */
const FIRST_RETRY_MS = 1_000;
const LAST_RETRY_MS = 30_000;

/** 连接需要的 `EventSource` 的部分，测试可以换成别的实现。 */
export interface Stream {
  addEventListener(type: string, listener: (event: MessageEvent) => void): void;
  onerror: ((event: Event) => void) | null;
  close(): void;
}

/** 测试可以用来替换浏览器所提供部分的选项。 */
export interface ConnectionOptions {
  open?: (url: string) => Stream;
  /** 网页和服务端在同一台机器上时为 true，这时断网不会断开它们。 */
  loopback?: boolean;
  random?: () => number;
  /** 服务端是否接受 token；实时消息的事件流（需要 token）失败时询问，见 `checkToken`。 */
  checkToken?: () => Promise<'ok' | 'unauthorized' | 'offline'>;
}

/** 第 `attempt` 次重试（从 0 开始）之前的等待：从 1 秒加倍到 30 秒，上下浮动五分之一，让各标签页错开。 */
export function retryDelay(attempt: number, random = Math.random): number {
  const base = Math.min(LAST_RETRY_MS, FIRST_RETRY_MS * 2 ** attempt);
  return Math.min(LAST_RETRY_MS, Math.round(base * (0.8 + 0.4 * random())));
}

const isLoopback = (host: string) => host === 'localhost' || host === '[::1]' || host === '::1' || /^127\./.test(host);

/**
 * 与服务端保持一个长连接的事件流，而不是一遍遍地询问（#221）。事件流一断（服务端停止或崩溃），状态立即变为
 * `offline`；超过看门狗时限仍没有任何事件的事件流（半开的连接：Wi-Fi 断了、NAT 忘了这个连接）也会被丢弃。
 * 之后隔 1、2、4……30 秒重新连接；网络或标签页回来时、调用 `reconnect()` 时立即重新连接。
 * 重新连接由这里自己做，不交给 `EventSource`：浏览器自己的重试无法控制节奏，而且遇到某些失败会永久停止。
 * 一段时间不可见或进入往返缓存的页面不保持事件流（#799，见 presence.ts）：状态保持不变，
 * 页面回来后事件流从最后一个事件接着读。
 */
export class Connection {
  private state: Backend = { state: 'checking' };
  private stream: Stream | null = null;
  private attempt = 0;
  private retry: ReturnType<typeof setTimeout> | undefined;
  private watchdog: ReturnType<typeof setTimeout> | undefined;
  private lastEvent = 0;
  private stopped = false;
  /** 实时消息（#201）：事件流换成 `/api/push/messages`，它同样发 `hello` 和心跳。 */
  private live = false;
  /** 最后一个消息事件的 ID，中断后从它接着读（`last-event-id`）。 */
  private lastEventId = '';
  /** 下一个 `hello` 是否是网页收到的第一个；第一个不需要补读中断期间的消息。 */
  private first = true;
  /**
   * 服务端拒绝了 token（#214）：在设置另一个 token 之前，状态保持 `unauthorized`，事件流用不需要 token 的
   * `/health/events`。
   */
  private unauthorized = false;
  private stopAuth: (() => void) | null = null;
  /** 页面此刻能否保持事件流（见 presence.ts）。 */
  private present = true;
  private stopPresence: (() => void) | null = null;
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
    this.stopPresence = onPresence(this.presence);
    this.present = present();
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    window.removeEventListener('online', this.online);
    window.removeEventListener('offline', this.offline);
    document.removeEventListener('visibilitychange', this.visible);
    this.stopAuth?.();
    this.stopAuth = null;
    this.stopPresence?.();
    this.stopPresence = null;
    this.drop();
  }

  /** 立即连接，等待时间从头算起（设置里的「重新检测」、网络恢复时）。 */
  reconnect(): void {
    this.attempt = 0;
    this.connect();
  }

  /** 接收实时消息（与服务端的状态共用一个事件流，#221 a），或者只接收服务端的状态。 */
  setLive(live: boolean): void {
    if (live === this.live) return;
    this.live = live;
    if (!this.stopped && this.stream) this.reconnect();
  }

  private url(): string {
    if (!this.live || this.unauthorized) return EVENTS_URL;
    // EventSource 不能带请求头：与媒体一样，token 放在地址里
    return apiUrl('api/push/messages', { access_token: token() ?? undefined, 'last-event-id': this.lastEventId || undefined });
  }

  /** 按服务端的回答得出的状态；服务端就绪但拒绝过 token 时为 `unauthorized`。 */
  private judged(health: Health): Backend {
    const state = stateOf(health);
    return state.state === 'ready' && this.unauthorized ? { state: 'unauthorized', health } : state;
  }

  // 任何请求收到 401，或者用户设置了另一个 token
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
    // 实时消息的事件流需要 token：设置另一个 token 之前只接收服务端的状态
    if (this.live && !this.stopped) this.reconnect();
  }

  private set(next: Backend): void {
    const now = this.state;
    const same = now.state === next.state && (next.state === 'checking' || next.state === 'offline'
      || ('health' in now && 'health' in next && JSON.stringify(now.health) === JSON.stringify(next.health)));
    if (same) return;
    this.state = next;
    // 网页跟随服务端的语言（#439）
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
    if (this.stopped || !this.present) return;
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
      // 中断后恢复，或者刚打开实时消息：这期间可能来过消息
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
          // 不带会话的事件没有可显示的内容
        }
      });
    }
    stream.addEventListener('heartbeat', event => {
      if (this.stream !== stream) return;
      heard();
      // 事件流没断但数据变了：换了账号（见 scope.ts）
      const now = this.state;
      if (!('health' in now)) return;
      try {
        const { data } = JSON.parse(event.data) as { data?: string };
        if (data !== undefined && data !== now.health.data) this.set(this.judged({ ...now.health, data }));
      } catch {
        // 不论内容是什么，心跳都说明事件流还活着
      }
    });
    // 服务端被主动停止（Ctrl+C）：与断开时一样立即变为 offline，服务端重新启动后连回
    stream.addEventListener('bye', () => {
      if (this.stream === stream) this.lost();
    });
    stream.onerror = () => {
      if (this.stream !== stream) return;
      this.lost();
      if (!this.live || this.unauthorized) return;
      // 实时消息的事件流需要 token，而 EventSource 无法区分 token 被拒绝（401）和服务端断开，因此询问一次。
      // token 被拒绝时立即换成只接收服务端状态的事件流，状态随之变为 `unauthorized`。
      this.checkToken().then(answer => {
        if (answer === 'unauthorized' && !this.stopped && !this.unauthorized) this.refused();
      }, () => { /* 没有回答：维持上面按断开所做的处理 */ });
    };
    heard();
  }

  /** 事件流断了或者长时间没有事件：立即变为 offline，等待之后再试。 */
  private lost(): void {
    this.drop();
    const now = this.state;
    this.set(now.state === 'ready' || (now.state === 'offline' && now.held) ? { state: 'offline', held: true } : { state: 'offline' });
    if (this.stopped) return;
    this.retry = setTimeout(() => this.connect(), retryDelay(this.attempt, this.random));
    this.attempt += 1;
  }

  private readonly online = () => this.reconnect();

  // 服务端在本机时断网它也还在：立即检查一次，但不因此显示为离线
  private readonly offline = () => {
    if (this.loopback) this.reconnect();
    else this.lost();
  };

  // 后台标签页的计时器会被放慢：回到前台时，除非一切正常，否则立即连接
  private readonly visible = () => {
    if (document.visibilityState !== 'visible' || !this.present) return;
    if (!('health' in this.state)) this.reconnect();
    else if (Date.now() - this.lastEvent > WATCHDOG_MS) this.reconnect();
  };

  // 页面离开时放开事件流，也不重试；回来时从最后一个事件接着读（`resumed` 让各处重读）
  private readonly presence = (here: boolean) => {
    this.present = here;
    if (here) this.reconnect();
    else this.drop();
  };
}

/**
 * 服务端的状态（来自一个事件流，见 `Connection`）和立即重新连接的函数 `recheck`。
 * `live` 为 true 时，事件流是实时消息的事件流（见 live.ts）。
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
