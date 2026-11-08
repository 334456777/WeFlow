// The few lines of the release notes from two benchmark results (#602), for release-bench-notes.sh.
// usage: node cli.mjs <base result.json> <head result.json> --out <dir>
// Writes <dir>/highlights.md; exits 2 when the two cannot be compared.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { highlights, incomparable } from './compare.mjs';

const args = process.argv.slice(2);
const at = args.indexOf('--out');
const out = at < 0 ? null : args.splice(at, 2)[1];
if (args.length !== 2 || !out) {
  console.error('usage: node cli.mjs <base result.json> <head result.json> --out <dir>');
  process.exit(1);
}
const [base, head] = args.map(path => JSON.parse(readFileSync(path, 'utf8')));
const why = incomparable(base, head);
if (why) {
  console.error(why);
  process.exit(2);
}
mkdirSync(out, { recursive: true });
writeFileSync(join(out, 'highlights.md'), highlights(base, head).map(line => `- ${line}\n`).join(''));
