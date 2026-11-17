// Two sets of shots of `bun run shots` compared pixel by pixel (#827). Pictures whose files are the same bytes are
// the same; the others are decoded in Chrome, which drew them, and each that differs leaves a picture of where: its
// changed pixels red over a pale copy of the second shot.
import { existsSync, mkdirSync, readdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import type { Browser } from './cdp.ts';

export interface Difference {
  file: string;
  /** Why the shots differ: pixels changed (with the box around them), another size, or one of them missing. */
  text: string;
}

const shotsIn = (dir: string) => new Set(readdirSync(dir).filter(name => name.endsWith('.png')));

/** The shots of `before` and `after` that differ; the pictures of where go into `out`. */
export async function compare(browser: Browser, before: string, after: string, out: string): Promise<{ same: number; differences: Difference[] }> {
  for (const dir of [before, after]) if (!existsSync(dir)) throw new Error(`no shots in ${dir}`);
  rmSync(out, { recursive: true, force: true });
  const a = shotsIn(before), b = shotsIn(after);
  const differences: Difference[] = [];
  let same = 0;
  const page = await browser.newPage();
  try {
    for (const file of [...new Set([...a, ...b])].sort()) {
      if (!b.has(file)) { differences.push({ file, text: 'only before' }); continue; }
      if (!a.has(file)) { differences.push({ file, text: 'only after' }); continue; }
      const [x, y] = await Promise.all([Bun.file(join(before, file)).bytes(), Bun.file(join(after, file)).bytes()]);
      if (x.length === y.length && x.every((byte, i) => byte === y[i])) { same += 1; continue; }
      const result = await page.eval(diff, x.toBase64(), y.toBase64());
      if (result.kind === 'size') {
        differences.push({ file, text: `size ${result.before} → ${result.after}` });
      } else if (result.changed === 0) {
        same += 1;
      } else {
        mkdirSync(out, { recursive: true });
        await Bun.write(join(out, file), Uint8Array.fromBase64(result.png));
        const [left, top, right, bottom] = result.box;
        differences.push({ file, text: `${result.changed} pixels changed in ${left},${top}–${right},${bottom}, by ${result.most} at most` });
      }
    }
  } finally {
    await page.close();
  }
  return { same, differences };
}

type Diff = { kind: 'size'; before: string; after: string }
  | { kind: 'pixels'; changed: number; box: [number, number, number, number]; most: number; png: string };

/** Runs in the page: the two PNGs (base64) compared, and the picture of where they differ. */
async function diff(before: string, after: string): Promise<Diff> {
  const decode = (data: string) => createImageBitmap(new Blob([Uint8Array.fromBase64(data)], { type: 'image/png' }),
    { colorSpaceConversion: 'none', premultiplyAlpha: 'none' });
  const [x, y] = await Promise.all([decode(before), decode(after)]);
  if (x.width !== y.width || x.height !== y.height) {
    return { kind: 'size', before: `${x.width}×${x.height}`, after: `${y.width}×${y.height}` };
  }
  const { width, height } = x;
  const canvas = new OffscreenCanvas(width, height);
  const context = canvas.getContext('2d', { willReadFrequently: true })!;
  const pixels = (bitmap: ImageBitmap) => {
    context.clearRect(0, 0, width, height);
    context.drawImage(bitmap, 0, 0);
    return context.getImageData(0, 0, width, height).data;
  };
  const p = pixels(x), q = pixels(y);
  const shown = context.createImageData(width, height);
  let changed = 0, most = 0, left = width, top = height, right = -1, bottom = -1;
  for (let i = 0; i < p.length; i += 4) {
    const delta = Math.max(Math.abs(p[i] - q[i]), Math.abs(p[i + 1] - q[i + 1]), Math.abs(p[i + 2] - q[i + 2]), Math.abs(p[i + 3] - q[i + 3]));
    if (delta) {
      changed += 1;
      most = Math.max(most, delta);
      const px = (i / 4) % width, py = Math.floor(i / 4 / width);
      left = Math.min(left, px); right = Math.max(right, px); top = Math.min(top, py); bottom = Math.max(bottom, py);
      shown.data.set([255, 0, 0, 255], i);
    } else {
      const grey = 200 + Math.round((q[i] * .3 + q[i + 1] * .59 + q[i + 2] * .11) * 55 / 255);
      shown.data.set([grey, grey, grey, 255], i);
    }
  }
  context.putImageData(shown, 0, 0);
  const png = new Uint8Array(await (await canvas.convertToBlob({ type: 'image/png' })).arrayBuffer()).toBase64();
  return { kind: 'pixels', changed, box: [left, top, right, bottom], most, png };
}
