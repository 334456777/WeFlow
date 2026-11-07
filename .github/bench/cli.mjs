// The comparison of two benchmark results from the command line (#602), for the run's page and the release notes.
// usage: node cli.mjs <base result.json> <head result.json> --out <dir> [--url <viewer url>]
// Writes <dir>/summary.md (the report and the memory curves as Mermaid charts) and <dir>/highlights.md (the few lines
// of the release notes); exits 2 when the two cannot be compared.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { highlights, incomparable, markdownCurves, reportBlocks, toMarkdown } from './compare.mjs';

const args = process.argv.slice(2);
const flag = name => {
  const i = args.indexOf(name);
  if (i < 0) return null;
  const [, value] = args.splice(i, 2);
  return value;
};
const out = flag('--out');
const url = flag('--url');
if (args.length !== 2 || !out) {
  console.error('usage: node cli.mjs <base result.json> <head result.json> --out <dir> [--url <viewer url>]');
  process.exit(1);
}
const [base, head] = args.map(path => JSON.parse(readFileSync(path, 'utf8')));
const why = incomparable(base, head);
if (why) {
  console.error(why);
  process.exit(2);
}
mkdirSync(out, { recursive: true });
const blocks = reportBlocks(base, head);
if (url) blocks.splice(1, 0, { p: `**[在查看页打开](${url})**` });
const markdown = toMarkdown(blocks) + '\n' + markdownCurves(base, head);
writeFileSync(join(out, 'summary.md'), markdown);
writeFileSync(join(out, 'highlights.md'), highlights(base, head).map(line => `- ${line}\n`).join(''));
console.log(markdown);
