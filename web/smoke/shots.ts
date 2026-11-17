// `bun run shots` (#827): screenshots of the built page (web/dist, `bun run build`) to tell whether a change of the
// styles changes what is drawn. The scenes of smoke/scenes.ts are shot in headless Chrome over a `weflow serve
// --ui-dir web/dist` of the synthetic account (scripts/account.ts), in the light and the dark theme, each in a tab of
// a browser context of its own; before a shot the page settles: its fonts and pictures loaded, the transitions over,
// the loops (a spinner, a loading sweep) stopped at their start.
//
//   bun run shots <name> [--skin <id>] [<word>...]
//                                       shots into target/web-shots/<name>/ (only the scenes with one of the words), in
//                                       the skin <id> of src/skin.ts (none: the default)
//   bun run shots --compare <a> <b>     <a> and <b> compared pixel by pixel; where they differ, in target/web-shots/<a>-vs-<b>/
//
// Shots differ from one machine to another (fonts, Chrome's version), so the two sets compared are taken on the same
// one, on the same day (dates are written relative to today), and served by the same weflow (pinned below).
//
// A set is in one skin, which its file `skin` names: a partial run in another skin into it fails, and --compare says
// when the two sets are in different skins.
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import { accounts, buildAccount, startServe, target } from '../scripts/account.ts';
import { Browser, findChrome, type Page } from './cdp.ts';
import { compare } from './compare.ts';
import { REST, scenes, type Scene, type Shoot } from './scenes.ts';

const DESKTOP = { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false };
const PHONE = { width: 390, height: 844, deviceScaleFactor: 1, mobile: true };
/** Tabs shot at once, each in a window of its own. */
const AT_ONCE = 3;
/** Drawn the same way every time: in software on one thread, in sRGB. Else the edge of a rounded shadow differs by a
 * shade at a pixel or two from one run to the next. */
const STEADY = ['--disable-gpu', '--num-raster-threads=1', '--disable-partial-raster', '--force-color-profile=srgb'];
/** Where the page keeps the skin chosen: skin.ts's SKIN_KEY, written again here since skin.ts cannot be loaded outside
 * a page (the i18n module it imports sets the document's title as it loads). */
const SKIN_KEY = 'weflow.skin';
/** The file of a set that names its skin. */
const SKIN_FILE = 'skin';

const out = join(target, 'web-shots');
const args = process.argv.slice(2);

if (args[0] === '--compare') {
  const [before, after] = args.slice(1);
  if (!before || !after) usage();
  const browser = await Browser.launch(findChrome(), join(out, 'profile'));
  try {
    const { same, differences } = await compare(browser, join(out, before), join(out, after), join(out, `${before}-vs-${after}`));
    const [a, b] = [before, after].map(set => skinOf(join(out, set)));
    if (a && b && a !== b) console.log(`${before} is in the skin ${a}, ${after} in ${b}`);
    for (const { file, text } of differences) console.log(`✗ ${file}: ${text}`);
    console.log(differences.length
      ? `${differences.length} of ${same + differences.length} shots differ; where: ${join(out, `${before}-vs-${after}`)}`
      : `all ${same} shots are the same`);
    process.exitCode = differences.length ? 1 : 0;
  } finally {
    await browser.close();
  }
} else {
  const [name, ...rest] = args;
  if (!name || !/^[\w-]+$/.test(name) || name === 'profile') usage();
  const at = rest.indexOf('--skin');
  const skin = at < 0 ? undefined : rest[at + 1];
  if (at >= 0 && !/^[\w-]+$/.test(skin ?? '')) usage();
  await shoot(name, at < 0 ? rest : rest.toSpliced(at, 2), skin).catch((error: Error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}

function usage(): never {
  console.error('usage: bun run shots <name> [--skin <id>] [<word>...] | bun run shots --compare <name> <name>');
  process.exit(2);
}

/** The skin the shots in `dir` are in; none for a set shot before sets named theirs. */
function skinOf(dir: string): string | undefined {
  const file = join(dir, SKIN_FILE);
  return existsSync(file) ? readFileSync(file, 'utf8').trim() : undefined;
}

type Variant = { scheme: 'light' | 'dark'; contrast: boolean };

async function shoot(name: string, only: string[], skin: string | undefined): Promise<void> {
  const dist = join(import.meta.dir, '../dist');
  if (!existsSync(join(dist, 'index.html'))) {
    console.error('no build in web/dist: run `bun run build` first');
    process.exit(1);
  }
  const dir = join(out, name);
  const jobs = scenes
    .filter(scene => !only.length || only.some(word => scene.name.includes(word)))
    .flatMap(scene => variants(scene).map(variant => ({ scene, variant })));
  if (!jobs.length) {
    console.error('no scene has any of the words ' + only.join(', '));
    process.exit(2);
  }

  const { weflow: built, home } = buildAccount(accounts.dev);
  const weflow = pinned(built);
  console.log('served by ' + Bun.spawnSync([weflow, '--version']).stdout.toString().trim() + ' (' + weflow + ')');
  const browser = await Browser.launch(findChrome(), join(out, 'profile'), STEADY);
  const { serve, url, token } = await startServe(weflow, home, ['--ui-dir', dist, '--no-browser'], { WEFLOW_LANG: 'zh' });
  const address = `${url}/ui/?token=${token}`;
  const started = performance.now();
  let failed = 0, shots = 0, next = 0;
  try {
    // the skin is tried in one tab first, so that one the page does not have fails once, before the set is touched
    const probe = await browser.newPage({ isolated: true });
    const shown = await open(probe, address, skin).finally(() => probe.close());
    const kept = skinOf(dir);
    // a full run starts afresh; a partial one replaces the shots of its scenes only, in the skin of the set
    if (only.length && kept && kept !== shown) throw new Error(`the shots in ${dir} are in the skin ${kept}: shoot ${shown} into a set of its own`);
    if (!only.length) rmSync(dir, { recursive: true, force: true });
    mkdirSync(dir, { recursive: true });
    for (const { scene, variant } of jobs) {
      for (const file of readdirSync(dir)) if (file.startsWith(scene.name + '.') && file.endsWith(`.${suffix(variant)}.png`)) rmSync(join(dir, file));
    }
    writeFileSync(join(dir, SKIN_FILE), shown + '\n');
    console.log('in the skin ' + shown);
    const worker = async () => {
      while (next < jobs.length) {
        const { scene, variant } = jobs[next++];
        try {
          const taken = await run(browser, address, skin, scene, variant, dir);
          shots += taken;
          console.log(`✓ ${scene.name} (${suffix(variant)})`);
        } catch (error) {
          failed += 1;
          console.error(`✗ ${scene.name} (${suffix(variant)})\n  ${(error as Error).message}`);
        }
      }
    };
    await Promise.all(Array.from({ length: AT_ONCE }, worker));
  } finally {
    serve.kill();
    await serve.exited;
    await browser.close();
  }
  console.log(`${shots} shots in ${dir} (${Math.round((performance.now() - started) / 1000)} s)` + (failed ? `; ${failed} of ${jobs.length} scenes failed` : ''));
  process.exitCode = failed ? 1 : 0;
}

/**
 * The weflow every set of shots is served by: a copy of the one built for the first set, kept in target/web-shots/
 * (deleted, the next set keeps a copy of its own). A weflow built again shows another version and build time in the
 * settings, which is no change of the page, and the shots are to compare the page only.
 */
function pinned(built: string): string {
  const path = join(out, basename(built));
  if (!existsSync(path)) copyFileSync(built, path);
  return path;
}

function variants(scene: Scene): Variant[] {
  const contrast = scene.contrast ? [false, true] : [false];
  return contrast.flatMap(contrast => (['light', 'dark'] as const).map(scheme => ({ scheme, contrast })));
}

function suffix(variant: Variant): string {
  return variant.scheme + (variant.contrast ? '-contrast' : '');
}

/**
 * Opens the page in `page` (a tab of a context of its own, where nothing is kept yet) in the skin `skin`, the default
 * for none, until it has connected to weflow serve: the skin it shows. Fails when that is not the skin asked for (one
 * src/skin.ts does not have, or whose sheet could not be fetched, leaves the default on).
 */
async function open(page: Page, address: string, skin: string | undefined): Promise<string> {
  // kept before the page's first script reads it: index.html puts the skin on before the first paint
  if (skin) await page.send('Page.addScriptToEvaluateOnNewDocument', { source: `localStorage.setItem(${JSON.stringify(SKIN_KEY)}, ${JSON.stringify(skin)});` });
  await page.goto(address);
  await page.waitFor('the page to connect to weflow serve', () => document.querySelector('.connection.ready') !== null);
  // main.tsx draws the page once the skin is on, so it is the one on now
  const shown = await page.eval(() => document.documentElement.dataset.skin ?? '');
  if (skin && shown !== skin) throw new Error(`the page shows the skin ${shown}, not ${skin}: no skin ${skin} in src/skin.ts, or its sheet could not be fetched`);
  return shown;
}

/** The scene in a new tab of its own context, in the skin `skin`: the number of shots it took. */
async function run(browser: Browser, address: string, skin: string | undefined, scene: Scene, variant: Variant, dir: string): Promise<number> {
  const page = await browser.newPage({ isolated: true });
  let taken = 0;
  try {
    await page.send('Emulation.setDeviceMetricsOverride', scene.phone ? PHONE : DESKTOP);
    await page.send('Emulation.setEmulatedMedia', {
      features: [
        { name: 'prefers-color-scheme', value: variant.scheme },
        { name: 'prefers-contrast', value: variant.contrast ? 'more' : 'no-preference' },
        { name: 'prefers-reduced-motion', value: 'no-preference' },
      ],
    });
    await open(page, address, skin);
    const shoot: Shoot = async (label, around) => {
      // the viewport is shot with the mouse off every control; a part of it, as the scene left the mouse
      if (!around) await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...REST });
      const left = await settle(page);
      if (left.length) console.warn(`  ${scene.name}${label ? ' ' + label : ''}: loading placeholders still on the page, in ${[...new Set(left)].join(', ')}`);
      const clip = around && await page.eval(({ selector, text, margin = 24 }) => {
        const element = [...document.querySelectorAll<HTMLElement>(selector)]
          .find(element => !text || (element.textContent ?? '').includes(text) || element.getAttribute('aria-label')?.includes(text));
        const box = element?.getBoundingClientRect();
        if (!box) return null;
        const x = Math.max(0, Math.floor(box.left - margin)), y = Math.max(0, Math.floor(box.top - margin));
        return { x, y, width: Math.min(innerWidth, Math.ceil(box.right + margin)) - x, height: Math.min(innerHeight, Math.ceil(box.bottom + margin)) - y };
      }, around);
      if (around && !clip) throw new Error(`nothing to shoot: no ${around.selector}`);
      await Bun.write(join(dir, `${scene.name}${label ? '.' + label : ''}.${suffix(variant)}.png`), await page.screenshot(clip || undefined));
      taken += 1;
    };
    await scene.run(page, shoot);
    return taken;
  } finally {
    await page.close();
  }
}

/**
 * Waits until nothing on the page is still changing on its own: the fonts and the pictures in view loaded, every
 * transition and finite animation over, no scrollbar showing for a scroll, the connection's label faded, no notice up,
 * and no loading placeholder left outside a folded card (or five seconds gone); the same twice in a row. Then the loops
 * are stopped at their start, a text field loses the focus (its caret blinks), and the page draws a frame. The loading
 * placeholders left (the class of what holds each): a shot with any is not to be trusted, as they may go at any time.
 */
async function settle(page: Page): Promise<string[]> {
  await page.eval(() => document.fonts.ready.then(() => true));
  await page.eval(() => {
    const field = document.activeElement;
    if (field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement) field.blur();
  });
  await calm(page);
  const deadline = Date.now() + 5_000;
  let placeholders: string[];
  // a folded card keeps its body, placeholders included, out of sight and reads nothing for it
  const left = () => [...document.querySelectorAll('.skeleton')].filter(element => !element.closest('.card.collapsed'))
    .map(element => element.parentElement?.className || element.className);
  while ((placeholders = await page.eval(left)).length && Date.now() < deadline) await Bun.sleep(100);
  await Bun.sleep(150);
  await calm(page);
  await page.eval(() => {
    for (const animation of document.getAnimations()) {
      if (animation.effect?.getComputedTiming().iterations !== Infinity) continue;
      animation.pause();
      animation.currentTime = 0;
    }
    return new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  });
  return placeholders;
}

/**
 * Waits up to thirty seconds (the pictures are decoded by a debug build, several tabs at once) until nothing changes on
 * its own; else fails with what still did. A picture out of view is left out: one loaded lazily never loads there.
 */
async function calm(page: Page): Promise<void> {
  const changing = () => {
    const name = (element: Element | null | undefined) => element ? element.tagName.toLowerCase() + '.' + [...element.classList].join('.') : '?';
    const loading = [...document.images].find(img => {
      const box = img.getBoundingClientRect();
      return !img.complete && box.bottom > 0 && box.top < innerHeight && box.right > 0 && box.left < innerWidth;
    });
    if (loading) return 'a picture loading: ' + loading.getAttribute('src')?.replace(/access_token=[^&]*/, 'access_token=…');
    const shown = document.querySelector('.scrollbar.active, .connection:not(.quiet), .toast');
    if (shown) return name(shown);
    const moving = document.getAnimations().find(a => a.effect?.getComputedTiming().iterations !== Infinity && (a.playState === 'running' || a.pending));
    if (moving) return `${(moving as CSSTransition).transitionProperty ?? (moving as CSSAnimation).animationName ?? 'an animation'} of ${name((moving.effect as KeyframeEffect | null)?.target)}`;
    return '';
  };
  const deadline = Date.now() + 30_000;
  for (;;) {
    const what = await page.eval(changing);
    if (!what) return;
    if (Date.now() > deadline) throw new Error('the page did not settle: ' + what);
    await Bun.sleep(100);
  }
}
