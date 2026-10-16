// Calls to `weflow serve` (docs/serve-api.md). Answers are `{"success": true, "data": ...}` or
// `{"success": false, "error": {"code", "message", "details"}}`; `call` returns the data or throws an ApiError.

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

/** The token for this tab: set by `adoptToken`, or null when the server needs none (the dev proxy adds it). */
export function token(): string | null {
  return memoryToken ?? storage()?.getItem(TOKEN_KEY) ?? null;
}

/**
 * Sets the token for this tab (#214): kept in `sessionStorage` only, never `localStorage` nor the build. The
 * connection hears of it and tries again.
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

/** `unauthorized`: the server refused the token (401); `token`: another token was set. */
export type AuthEvent = 'unauthorized' | 'token';
const authListeners = new Set<(event: AuthEvent) => void>();

export function onAuth(listener: (event: AuthEvent) => void): () => void {
  authListeners.add(listener);
  return () => { authListeners.delete(listener); };
}

function tellAuth(event: AuthEvent): void {
  for (const listener of [...authListeners]) listener(event);
}

/**
 * Whether the server takes this tab's token, asked with a light route that needs one (`/jobs`): `ok`,
 * `unauthorized` (401), or `offline` (no answer). An event stream cannot tell why it failed; this can.
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
 * Takes the token from the address `weflow serve` opens (`/ui/?token=...`, #146), keeps it for this tab only and
 * removes it from the address bar and the history.
 */
export function adoptToken(): void {
  const url = new URL(window.location.href);
  const value = url.searchParams.get('token');
  if (!value) return;
  setToken(value);
  url.searchParams.delete('token');
  window.history.replaceState(window.history.state, '', url.pathname + url.search + url.hash);
}

/** `path` with the parameters as a query; list values are repeated (`?id=a&id=b`), empty ones left out. */
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

type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

/** Runs a route: GET and DELETE take the parameters as a query, the other methods as a JSON body. */
export async function call<T>(method: Method, path: string, params: Params = {}, signal?: AbortSignal): Promise<T> {
  const revision = tokenRevision;
  const inQuery = method === 'GET' || method === 'DELETE';
  const headers: Record<string, string> = { Accept: 'application/json' };
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
    throw new ApiError(0, 'offline', '无法连接 weflow serve');
  }
  // An old request may finish after the person has already supplied the right token.
  if (response.status === 401 && revision === tokenRevision && !signal?.aborted) tellAuth('unauthorized');

  let doc: { success?: boolean; data?: T; error?: { code?: string; message?: string; details?: unknown } };
  try {
    doc = await response.json();
  } catch {
    throw new ApiError(response.status, 'bad_response', '回复不是 JSON（HTTP ' + response.status + '）');
  }
  if (response.ok && doc.success !== false) return doc.data as T;
  const error = doc.error ?? {};
  throw new ApiError(response.status, error.code ?? 'http_' + response.status, error.message ?? response.statusText, error.details);
}

export const get = <T>(path: string, params?: Params, signal?: AbortSignal) => call<T>('GET', path, params, signal);
