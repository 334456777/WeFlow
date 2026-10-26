// Whether a client written against an earlier contract reads the current one's answers right (#400):
// `bun scripts/contract-compat.ts <earlier openapi.json>` lists what it could read wrong, and fails when there is
// something and `info.version` (the API version) is the same; CI compares a change with the contract of its base.
// It reads the schemas schemars writes, as scripts/contract.ts does.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

type Schema = { [key: string]: unknown } | boolean;
interface Doc {
  info: { version: string };
  paths: Record<string, { get: { responses: Record<string, { content?: Record<string, { schema: Schema }> }> } }>;
  components: { schemas: Record<string, Schema> };
}

const REF = '#/components/schemas/';

const answerOf = (doc: Doc, path: string) => doc.paths[path]?.get.responses['200']?.content?.['application/json']?.schema;

/** The schema itself, through its `$ref`, and the name it was reached by. */
function resolve(doc: Doc, s: Schema): [Schema, string] {
  if (typeof s === 'object' && typeof s.$ref === 'string' && s.$ref.startsWith(REF)) {
    const name = s.$ref.slice(REF.length);
    return [doc.components.schemas[name] ?? false, name];
  }
  return [s, ''];
}

const branches = (s: Schema): Schema[] | null =>
  typeof s === 'object' ? ((s.anyOf ?? s.oneOf) as Schema[] | undefined) ?? null : null;

const kindsOf = (s: { [key: string]: unknown }): string[] | null =>
  s.type === undefined ? null : Array.isArray(s.type) ? s.type : [s.type as string];

const valuesOf = (s: { [key: string]: unknown }): unknown[] | null =>
  'const' in s ? [s.const] : Array.isArray(s.enum) ? s.enum : null;

const kindOfValue = (v: unknown) =>
  v === null ? 'null' : Array.isArray(v) ? 'array' : Number.isInteger(v) ? 'integer' : typeof v === 'number' ? 'number' : typeof v;

const accepts = (kinds: string[], kind: string) => kinds.includes(kind) || (kind === 'integer' && kinds.includes('number'));

/**
 * What `now` (of the current contract) may answer that `old` (of the earlier one) did not allow, as `where: what`.
 * Pairs of named schemas already being compared count as fitting, so that a recursive type ends.
 */
function misfits(docs: { now: Doc; old: Doc }, now: Schema, old: Schema, where: string, seen: Set<string>): string[] {
  const [n, nName] = resolve(docs.now, now);
  const [o, oName] = resolve(docs.old, old);
  if (nName || oName) {
    const pair = nName + '|' + oName;
    if (seen.has(pair)) return [];
    seen = new Set(seen).add(pair);
  }
  if (o === true) return [];
  if (o === false) return n === false ? [] : [where + ': had no value'];
  if (n === false) return [];
  if (n === true) return [where + ': may be any value'];

  const nb = branches(n);
  const ob = branches(o);
  // the same alternatives, in the order the code writes them: each against its own, as a parsed answer would fit
  // the alternative of raw rows too, which takes any object
  if (nb && ob && nb.length === ob.length) return nb.flatMap((b, i) => misfits(docs, b, ob[i], `${where}|${i}`, seen));
  if (nb) return nb.flatMap((b, i) => misfits(docs, b, o, `${where}|${i}`, seen));
  if (ob) {
    const each = ob.map(b => misfits(docs, n, b, where, seen));
    return each.some(p => p.length === 0) ? [] : [where + ': fits none of what it was: ' + each.flat().join('; ')];
  }

  const problems: string[] = [];
  const nValues = valuesOf(n);
  const oValues = valuesOf(o);
  if (oValues) {
    if (!nValues) return [where + ': is no longer one of ' + JSON.stringify(oValues)];
    for (const v of nValues) if (!oValues.some(w => JSON.stringify(w) === JSON.stringify(v))) problems.push(where + ': may be ' + JSON.stringify(v));
    return problems;
  }
  const oKinds = kindsOf(o);
  const nKinds = nValues ? [...new Set(nValues.map(kindOfValue))] : kindsOf(n);
  if (!oKinds) return [];
  if (!nKinds) return [where + ': may be any type, was ' + oKinds.join(' or ')];
  for (const k of nKinds) if (!accepts(oKinds, k)) problems.push(`${where}: may be ${k}, was ${oKinds.join(' or ')}`);
  for (const bound of ['minimum', 'maximum'] as const) {
    if (typeof o[bound] !== 'number') continue;
    const was = o[bound] as number;
    const is = n[bound];
    const wider = typeof is !== 'number' || (bound === 'minimum' ? is < was : is > was);
    if (wider && nKinds.some(k => k === 'integer' || k === 'number')) problems.push(`${where}: has no ${bound} ${was} any more`);
  }

  if (nKinds.includes('array') && oKinds.includes('array') && n.items !== undefined && o.items !== undefined) {
    problems.push(...misfits(docs, n.items as Schema, o.items as Schema, where + '[]', seen));
  }
  if (nKinds.includes('object') && oKinds.includes('object')) {
    const nProps = (n.properties ?? {}) as Record<string, Schema>;
    const oProps = (o.properties ?? {}) as Record<string, Schema>;
    const nRequired = new Set((n.required ?? []) as string[]);
    for (const key of (o.required ?? []) as string[]) if (!nRequired.has(key)) problems.push(`${where}.${key}: may be left out`);
    for (const [key, p] of Object.entries(nProps)) {
      if (key in oProps) problems.push(...misfits(docs, p, oProps[key], `${where}.${key}`, seen));
      else if (o.additionalProperties === false) problems.push(`${where}.${key}: is new where no other key was allowed`);
      else if (typeof o.additionalProperties === 'object') problems.push(...misfits(docs, p, o.additionalProperties as Schema, `${where}.${key}`, seen));
    }
  }
  return problems;
}

/** What a client of `old` could read wrong in the answers of `now`. */
export function breaks(old: Doc, now: Doc): string[] {
  const problems: string[] = [];
  for (const path of Object.keys(old.paths)) {
    const was = answerOf(old, path);
    const is = answerOf(now, path);
    if (was === undefined) continue;
    if (is === undefined) problems.push(path + ': is no longer described');
    else problems.push(...misfits({ now, old }, is, was, path, new Set()));
  }
  return problems;
}

if (import.meta.main) {
  const earlier = process.argv[2];
  if (!earlier) {
    console.error('usage: bun scripts/contract-compat.ts <earlier contract/openapi.json>');
    process.exit(2);
  }
  const read = (path: string) => JSON.parse(readFileSync(path, 'utf8')) as Doc;
  const old = read(earlier);
  const now = read(fileURLToPath(new URL('../contract/openapi.json', import.meta.url)));
  const problems = breaks(old, now);
  const same = old.info.version === now.info.version;
  if (!problems.length) {
    console.log(`the answers of API ${now.info.version} read as those of API ${old.info.version} did`);
  } else if (same) {
    console.error(`a client of API ${old.info.version} could read these answers wrong; raise apiVersion (docs/serve-api.md, "Versions"):`);
    for (const p of problems) console.error('  ' + p);
    process.exit(1);
  } else {
    console.log(`API ${old.info.version} → ${now.info.version}, which changes:`);
    for (const p of problems) console.log('  ' + p);
  }
}
