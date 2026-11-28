// 命令控制台（#217）：`/openapi.json` 描述的每条命令，按它的参数生成表单，像服务端那样运行，
// 原样显示到来的回答（状态码、退出码和文档），以及它对应的命令行。
import { apiUrl, token, type Params } from './client';
import { t } from '../i18n';

/** 命令 API 用到的 HTTP 方法。 */
export type Method = 'GET' | 'POST' | 'PUT' | 'DELETE';

/** 命令的一个参数，即控制台表单里的一项。 */
export interface Field {
  name: string;
  type: 'string' | 'integer' | 'number' | 'boolean' | 'list' | 'choice';
  /** 选项的可选值。 */
  options?: string[];
  /** 列表的每一项是整数而不是文字。 */
  integers?: boolean;
  required: boolean;
  description: string;
  defaultValue?: string;
}

/** `/openapi.json` 里的一条命令，即控制台列出的一项。 */
export interface ConsoleCommand {
  /** 命令名，如 `chat sessions`。 */
  command: string;
  /** 命令的操作 ID，各不相同：方法由参数决定的命令（如 `chat group-hint`），每种方法各有一个。 */
  id: string;
  path: string;
  method: Method;
  tag: string;
  summary: string;
  description: string;
  fields: Field[];
  /** 命令会写出服务端可以交回的文件（`return=file`）。 */
  file: boolean;
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

const METHODS: Method[] = ['GET', 'POST', 'PUT', 'DELETE'];

/** `weflow serve` 的 OpenAPI 文档里的命令，按命令名排序（同名的按方法）；不含服务端自己的路由。 */
export function commandsOf(doc: unknown): ConsoleCommand[] {
  const paths = (doc as { paths?: Record<string, Record<string, Record<string, unknown>>> })?.paths ?? {};
  const out: ConsoleCommand[] = [];
  for (const [path, ops] of Object.entries(paths)) {
    // 进程内的读取（`/api/…`）是同样的命令，不重复列出
    if (path.startsWith('/api/')) continue;
    for (const [verb, raw] of Object.entries(ops)) {
      const method = verb.toUpperCase() as Method;
      const op = raw as {
        operationId?: string; tags?: string[]; summary?: string; description?: string; 'x-weflow-command'?: string;
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
        file: (op.parameters ?? []).some(p => p.$ref?.endsWith('/return')),
      });
    }
  }
  return out.sort((a, b) => a.command.localeCompare(b.command) || METHODS.indexOf(a.method) - METHODS.indexOf(b.method));
}

/**
 * 读取 `/openapi.json` 并列出其中的命令。它和所有回答一样用服务端的语言（#433），需要 token，
 * 而且不是 `{success, data}` 格式的文档。
 */
export async function getOpenApi(signal?: AbortSignal): Promise<ConsoleCommand[]> {
  const bearer = token();
  const response = await fetch('/openapi.json', { headers: { Accept: 'application/json', ...(bearer ? { Authorization: 'Bearer ' + bearer } : {}) }, signal });
  if (!response.ok) throw new Error(response.status === 401 ? t('令牌不对，读不到命令列表。') : t('读不到命令列表（HTTP {status}）。', { status: response.status }));
  return commandsOf(await response.json());
}

/** 把表单里填的值换成请求的参数：空的不要，列表按换行或逗号拆开。 */
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

/** 表单缺少或填错的地方，每处一行；可以运行时为空。 */
export function formProblems(cmd: ConsoleCommand, values: Record<string, string | boolean>): string[] {
  const problems: string[] = [];
  for (const field of cmd.fields) {
    const value = values[field.name];
    const text = typeof value === 'string' ? value.trim() : '';
    if (field.required && field.type !== 'boolean' && !text) problems.push(t('请填写 {name}。', { name: field.name }));
    if (text && field.type === 'integer' && !/^-?\d+$/.test(text)) problems.push(t('{name} 要填整数。', { name: field.name }));
    if (text && field.type === 'number' && !Number.isFinite(Number(text))) problems.push(t('{name} 要填数字。', { name: field.name }));
    if (text && field.type === 'list' && field.integers && text.split(/[\n,，]/).some(s => s.trim() && !/^-?\d+$/.test(s.trim()))) problems.push(t('{name} 的每一项都要是整数。', { name: field.name }));
  }
  return problems;
}

/** `?std=in`：请求会运行的命令行，不实际运行；回答就是 `{"argv", "stdin"}`，不是文档。 */
export async function commandLine(cmd: ConsoleCommand, params: Params, signal?: AbortSignal): Promise<string[] | null> {
  const answer = await runRaw(cmd, params, 'in', signal);
  const argv = (answer.body as { argv?: unknown } | null)?.argv;
  return answer.status === 200 && Array.isArray(argv) ? argv.map(String) : null;
}

/** 可以复制到终端的命令行：含空格或引号的词加上引号。 */
export const shellLine = (argv: string[]) => argv.map(a => (/^[\w@%+=:,./-]+$/.test(a) ? a : "'" + a.replace(/'/g, "'\\''") + "'")).join(' ');

/** 从控制台运行的命令的回答，以及用了多长时间。 */
export interface RunAnswer {
  status: number;
  exitCode: string | null;
  /** 回答的文档；不是 JSON 时为文字。 */
  body: unknown;
  ms: number;
}

/** 原样运行一条命令，不论服务端回答什么都原样返回。 */
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
  try { body = JSON.parse(text); } catch { /* 按文字显示 */ }
  return { status: response.status, exitCode: response.headers.get('X-WeFlow-Exit-Code'), body, ms: Math.round(performance.now() - started) };
}
