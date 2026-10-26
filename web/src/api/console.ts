// The command console (#217): every command `/openapi.json` describes, as a form from its parameters, run as the
// server runs it, with the answer shown as it comes (status, exit code and document), and the command line it is.
import { apiUrl, token, type Params } from './client';

export type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

export interface Field {
  name: string;
  type: 'string' | 'integer' | 'number' | 'boolean' | 'list' | 'choice';
  /** The values of a choice. */
  options?: string[];
  /** A list of integers rather than words. */
  integers?: boolean;
  required: boolean;
  description: string;
  defaultValue?: string;
}

export interface ConsoleCommand {
  /** `chat sessions`. */
  command: string;
  /** Its operation, unique: a command whose method follows an argument has one per method. */
  id: string;
  path: string;
  method: Method;
  tag: string;
  summary: string;
  description: string;
  fields: Field[];
  /** It writes a file the server can hand back (`return=file`). */
  file: boolean;
  /** WeChat's databases are read-only: the command always answers 501 (`x-weflow-refused`). */
  refused: boolean;
}

interface Schema { type?: string; enum?: unknown[]; items?: Schema; default?: unknown; description?: string }

function fieldOf(name: string, schema: Schema, required: boolean, description?: string): Field {
  const base = { name, required, description: description ?? schema.description ?? '', defaultValue: schema.default === undefined ? undefined : String(schema.default) };
  if (schema.enum) return { ...base, type: 'choice', options: schema.enum.map(String) };
  switch (schema.type) {
    case 'boolean': return { ...base, type: 'boolean' };
    case 'integer': return { ...base, type: 'integer' };
    case 'number': return { ...base, type: 'number' };
    case 'array': return { ...base, type: 'list', integers: schema.items?.type === 'integer' };
    default: return { ...base, type: 'string' };
  }
}

const METHODS: Method[] = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'];

/** The commands of an OpenAPI document of `weflow serve`, by tag and name; the server's own routes left out. */
export function commandsOf(doc: unknown): ConsoleCommand[] {
  const paths = (doc as { paths?: Record<string, Record<string, Record<string, unknown>>> })?.paths ?? {};
  const out: ConsoleCommand[] = [];
  for (const [path, ops] of Object.entries(paths)) {
    // the in-process reads (`/api/…`) are the same commands again
    if (path.startsWith('/api/')) continue;
    for (const [verb, raw] of Object.entries(ops)) {
      const method = verb.toUpperCase() as Method;
      const op = raw as {
        operationId?: string; tags?: string[]; summary?: string; description?: string; 'x-weflow-command'?: string; 'x-weflow-refused'?: boolean;
        parameters?: { name?: string; $ref?: string; required?: boolean; schema?: Schema; description?: string }[];
        requestBody?: { content?: { 'application/json'?: { schema?: { properties?: Record<string, Schema>; required?: string[] } } } };
      };
      const command = op['x-weflow-command'];
      if (!command || !METHODS.includes(method)) continue;
      const fields: Field[] = [];
      for (const p of op.parameters ?? []) {
        if (p.$ref || !p.name) continue;
        fields.push(fieldOf(p.name, p.schema ?? {}, Boolean(p.required), p.description));
      }
      const body = op.requestBody?.content?.['application/json']?.schema;
      for (const [name, schema] of Object.entries(body?.properties ?? {})) fields.push(fieldOf(name, schema, Boolean(body?.required?.includes(name))));
      out.push({
        command, id: op.operationId ?? command, path, method, tag: op.tags?.[0] ?? command.split(' ')[0],
        summary: op.summary ?? '', description: op.description ?? '', fields,
        file: (op.parameters ?? []).some(p => p.$ref?.endsWith('/return')), refused: Boolean(op['x-weflow-refused']),
      });
    }
  }
  return out.sort((a, b) => a.command.localeCompare(b.command) || METHODS.indexOf(a.method) - METHODS.indexOf(b.method));
}

/**
 * `/openapi.json`, in the server's language as every answer is (#433); it needs the token, and is no
 * `{success, data}` document.
 */
export async function getOpenApi(signal?: AbortSignal): Promise<ConsoleCommand[]> {
  const bearer = token();
  const response = await fetch('/openapi.json', { headers: { Accept: 'application/json', ...(bearer ? { Authorization: 'Bearer ' + bearer } : {}) }, signal });
  if (!response.ok) throw new Error(response.status === 401 ? '令牌不对，读不到命令列表。' : '读不到命令列表（HTTP ' + response.status + '）。');
  return commandsOf(await response.json());
}

/** The values typed in a form, as the parameters of the request: empty ones left out, lists split by line or comma. */
export function paramsOf(cmd: ConsoleCommand, values: Record<string, string | boolean>): Params {
  const params: Params = {};
  for (const field of cmd.fields) {
    const value = values[field.name];
    if (field.type === 'boolean') { if (value === true) params[field.name] = true; continue; }
    const text = typeof value === 'string' ? value.trim() : '';
    if (!text) continue;
    if (field.type === 'list') {
      const items = text.split(/[\n,，]/).map(s => s.trim()).filter(Boolean);
      params[field.name] = field.integers ? items.map(Number) : items;
    } else if (field.type === 'integer' || field.type === 'number') params[field.name] = Number(text);
    else params[field.name] = text;
  }
  return params;
}

/** What a form misses or holds wrongly, one line each; empty when it can run. */
export function formProblems(cmd: ConsoleCommand, values: Record<string, string | boolean>): string[] {
  const problems: string[] = [];
  for (const field of cmd.fields) {
    const value = values[field.name];
    const text = typeof value === 'string' ? value.trim() : '';
    if (field.required && field.type !== 'boolean' && !text) problems.push('请填写 ' + field.name + '。');
    if (text && field.type === 'integer' && !/^-?\d+$/.test(text)) problems.push(field.name + ' 要填整数。');
    if (text && field.type === 'number' && !Number.isFinite(Number(text))) problems.push(field.name + ' 要填数字。');
    if (text && field.type === 'list' && field.integers && text.split(/[\n,，]/).some(s => s.trim() && !/^-?\d+$/.test(s.trim()))) problems.push(field.name + ' 的每一项都要是整数。');
  }
  return problems;
}

/** `?std=in`: the command line the request runs, without running it; `{"argv", "stdin"}` itself, no document. */
export async function commandLine(cmd: ConsoleCommand, params: Params, signal?: AbortSignal): Promise<string[] | null> {
  const answer = await runRaw(cmd, params, 'in', signal);
  const argv = (answer.body as { argv?: unknown } | null)?.argv;
  return answer.status === 200 && Array.isArray(argv) ? argv.map(String) : null;
}

/** A command line to copy into a terminal: words with spaces or quotes are quoted. */
export const shellLine = (argv: string[]) => argv.map(a => (/^[\w@%+=:,./-]+$/.test(a) ? a : "'" + a.replace(/'/g, "'\\''") + "'")).join(' ');

export interface RunAnswer {
  status: number;
  exitCode: string | null;
  /** The document, or the text when it is none. */
  body: unknown;
  ms: number;
}

/** Runs a command as it is, and answers what the server answered, whatever it was. */
export async function runRaw(cmd: ConsoleCommand, params: Params, std?: 'in' | 'out' | 'err', signal?: AbortSignal): Promise<RunAnswer> {
  const inQuery = cmd.method === 'GET' || cmd.method === 'DELETE';
  const bearer = token();
  const started = performance.now();
  const query = { ...(inQuery ? params : {}), ...(std ? { std } : {}) };
  const response = await fetch(apiUrl(cmd.path, query), {
    method: cmd.method, signal,
    headers: { Accept: 'application/json', ...(bearer ? { Authorization: 'Bearer ' + bearer } : {}), ...(inQuery ? {} : { 'Content-Type': 'application/json' }) },
    body: inQuery ? undefined : JSON.stringify(params),
  });
  const text = await response.text();
  let body: unknown = text;
  try { body = JSON.parse(text); } catch { /* shown as text */ }
  return { status: response.status, exitCode: response.headers.get('X-WeFlow-Exit-Code'), body, ms: Math.round(performance.now() - started) };
}
