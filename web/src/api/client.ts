// 调用 `weflow serve`（见 Wiki 的「命令 API」页）。回答是 `{"success": true, "data": ...}` 或
// `{"success": false, "error": {"code", "message", "details"}}`；`call` 返回其中的数据，或者抛出 ApiError。
import { t } from '../i18n';

/** 失败的调用：HTTP 状态码、回答里的 `error.code`、错误信息和 `details`。 */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly details?: unknown,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

/** 调用的参数：GET 和 DELETE 放在查询参数里，其他方法放在 JSON 请求体里。 */
export type Params = Record<string, string | number | boolean | null | undefined | ReadonlyArray<string | number>>;

const TOKEN_KEY = 'weflow.token';
let memoryToken: string | null = null;
let tokenRevision = 0;

function storage(): Storage | null {
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
}

/** 这个标签页的 token：由 `adoptToken` 设置；不需要时为 null（开发服务器的代理会加上 token）。 */
export function token(): string | null {
  return memoryToken ?? storage()?.getItem(TOKEN_KEY) ?? null;
}

/**
 * 设置这个标签页的 token（#214）：只保存在 `sessionStorage` 里，不放进 `localStorage`，也不写进构建产物。
 * 连接会收到通知并重试。
 */
export function setToken(value: string | null): void {
  tokenRevision += 1;
  memoryToken = value;
  const s = storage();
  if (s) {
    if (value) s.setItem(TOKEN_KEY, value);
    else s.removeItem(TOKEN_KEY);
  }
  tellAuth('token');
}

/** 认证事件：`unauthorized` 表示服务端拒绝了 token（401），`token` 表示设置了另一个 token。 */
export type AuthEvent = 'unauthorized' | 'token';
const authListeners = new Set<(event: AuthEvent) => void>();

/** 每个认证事件都调用 `listener`；返回停止监听的函数。 */
export function onAuth(listener: (event: AuthEvent) => void): () => void {
  authListeners.add(listener);
  return () => { authListeners.delete(listener); };
}

function tellAuth(event: AuthEvent): void {
  for (const listener of [...authListeners]) listener(event);
}

/**
 * 用一个需要 token 的轻量路由（`/jobs`）询问服务端是否接受这个标签页的 token：`ok`、`unauthorized`（401）或
 * `offline`（没有回答）。事件流失败时无法得知原因，这个请求可以。
 */
export async function checkToken(signal?: AbortSignal): Promise<'ok' | 'unauthorized' | 'offline'> {
  const bearer = token();
  const revision = tokenRevision;
  try {
    const response = await fetch('/jobs', { headers: bearer ? { Authorization: 'Bearer ' + bearer } : {}, cache: 'no-store', signal });
    if (revision !== tokenRevision || signal?.aborted) return 'offline';
    if (response.status === 401) {
      tellAuth('unauthorized');
      return 'unauthorized';
    }
    return 'ok';
  } catch (problem) {
    if (problem instanceof DOMException && problem.name === 'AbortError') throw problem;
    return 'offline';
  }
}

/**
 * 从 `weflow serve` 打开的地址（`/ui/?token=...`，#146）取出 token，只为这个标签页保存，
 * 并从地址栏和浏览历史里去掉它。
 */
export function adoptToken(): void {
  const url = new URL(window.location.href);
  const value = url.searchParams.get('token');
  if (!value) return;
  setToken(value);
  url.searchParams.delete('token');
  window.history.replaceState(window.history.state, '', url.pathname + url.search + url.hash);
}

/** 带上查询参数的 `path`：列表的每个值各写一次（`?id=a&id=b`），空的参数不写。 */
export function apiUrl(path: string, params: Params = {}): string {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === '') continue;
    if (Array.isArray(value)) for (const item of value) query.append(key, String(item));
    else query.append(key, String(value));
  }
  const tail = query.toString();
  return '/' + path.replace(/^\/+/, '') + (tail ? '?' + tail : '');
}

/** 命令 API 用到的 HTTP 方法。 */
export type Method = 'GET' | 'POST' | 'PUT' | 'DELETE';

/**
 * 示例服务端对一个路由的回答：数据；错误，写成服务端的状态码和 `error`；或者 `refused`，表示它没有这个路由。
 * （示例服务端的 chunk 不导入页面的任何代码，所以它不自己构造 ApiError。）
 */
export type SampleAnswer =
  | { data: unknown }
  | { failed: { status: number; code: string; message: string } }
  | { refused: true };

/**
 * 代替服务端回答网页的示例服务端（#879）：`src/mock` 里的示例数据，在尚无服务端回答过这个网页，
 * 或者连上的服务端不适合它时使用。网页把它作为单独的 chunk 加载，并用 `setSample` 设置。
 * 改动只在页面的内存里生效，作业也在页面里运行（#896）。
 */
export interface SampleServer {
  /** 像 `weflow serve` 那样回答一个路由；请求希望作为作业运行时 `async` 为 true。 */
  answer(method: Method, path: string, params: Params, async: boolean): SampleAnswer;
  /** 示例里的媒体文件或作业文件的地址，对应服务端上 `path` 和 `params` 所指的文件。 */
  media(path: string, params: Params): string;
}

interface Sampling { server: Promise<SampleServer>; loaded: SampleServer | null }

let sample: Sampling | null = null;

/** 用 `load` 给出的示例服务端回答网页；为 null 时换回服务端。 */
export function setSample(load: (() => Promise<SampleServer>) | null): void {
  if (!load) { sample = null; return; }
  if (sample) return;
  const entry: Sampling = { server: load(), loaded: null };
  entry.server.then(server => { entry.loaded = server; }, () => { /* 每次调用各自报告失败 */ });
  sample = entry;
}

/** 网页现在是否由示例服务端回答。 */
export const sampling = () => sample !== null;

/** 媒体文件（`<img>`、`<audio>` 等）的地址：服务端上的地址；示例服务端回答网页时为示例里的地址。 */
export function mediaAddress(path: string, params: Params): string {
  return sample?.loaded ? sample.loaded.media(path, params) : apiUrl(path, params);
}

async function answerSample<T>(server: Promise<SampleServer>, method: Method, path: string, params: Params, async: boolean, signal?: AbortSignal): Promise<T> {
  const answering = await server;
  signal?.throwIfAborted();
  const answer = answering.answer(method, path.replace(/^\/+/, ''), params, async);
  if ('refused' in answer) throw new ApiError(0, 'sample', t('这是示例数据，连接 weflow serve 后才能这样做。'));
  if ('failed' in answer) throw new ApiError(answer.failed.status, answer.failed.code, answer.failed.message);
  // 回答归页面所有，与从服务端读到的一样：页面怎样改它都不影响示例数据
  return structuredClone(answer.data) as T;
}

/** 调用一个路由：GET 和 DELETE 把参数放在查询参数里，其他方法放在 JSON 请求体里。 */
export async function call<T>(method: Method, path: string, params: Params = {}, signal?: AbortSignal, extra: Record<string, string> = {}): Promise<T> {
  if (sample) return answerSample<T>(sample.server, method, path, params, extra.Prefer === 'respond-async', signal);
  const revision = tokenRevision;
  const inQuery = method === 'GET' || method === 'DELETE';
  const headers: Record<string, string> = { Accept: 'application/json', ...extra };
  const bearer = token();
  if (bearer) headers.Authorization = 'Bearer ' + bearer;
  if (!inQuery) headers['Content-Type'] = 'application/json';

  let response: Response;
  try {
    response = await fetch(apiUrl(path, inQuery ? params : {}), {
      method,
      headers,
      body: inQuery ? undefined : JSON.stringify(params),
      signal,
    });
  } catch (problem) {
    if (problem instanceof DOMException && problem.name === 'AbortError') throw problem;
    throw new ApiError(0, 'offline', t('无法连接 weflow serve'));
  }
  // 旧的请求可能在用户已经填了正确的 token 之后才结束，这时收到的 401 不发通知
  if (response.status === 401 && revision === tokenRevision && !signal?.aborted) tellAuth('unauthorized');

  let doc: { success?: boolean; data?: T; error?: { code?: string; message?: string; details?: unknown } };
  try {
    doc = await response.json();
  } catch {
    throw new ApiError(response.status, 'bad_response', t('回复不是 JSON（HTTP {status}）', { status: response.status }));
  }
  if (response.ok && doc.success !== false) return doc.data as T;
  const error = doc.error ?? {};
  throw new ApiError(response.status, error.code ?? 'http_' + response.status, error.message ?? response.statusText, error.details);
}

/** 用 GET 的 `call`，即一次读取。 */
export const get = <T>(path: string, params?: Params, signal?: AbortSignal) => call<T>('GET', path, params, signal);
