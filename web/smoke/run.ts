// `bun run smoke` (#789): the built page (web/dist, `bun run build`), rendered by Preact as it is released, tried in
// headless Chrome over a `weflow serve --ui-dir web/dist` of the synthetic account (scripts/account.ts): the checks of
// smoke/checks.ts on a server in Chinese, then those in English, each in a tab of its own, closed after it. A check
// that fails leaves a screenshot in target/web-smoke/. Chrome is found as smoke/cdp.ts says (CHROME, else where it is
// usually installed). `bun run smoke <word>...` runs only the checks whose name has one of the words.
import { existsSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { accounts, buildAccount, startServe, target } from '../scripts/account.ts';
import { Browser, findChrome, type Page, type Problem } from './cdp.ts';
import { checks, englishChecks, type Check } from './checks.ts';

const only = process.argv.slice(2);
const dist = join(import.meta.dir, '../dist');
const out = join(target, 'web-smoke');
if (!existsSync(join(dist, 'index.html'))) {
  console.error('no build in web/dist: run `bun run build` first');
  process.exit(1);
}
mkdirSync(out, { recursive: true });

const DESKTOP = { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false };
const PHONE = { width: 390, height: 844, deviceScaleFactor: 1, mobile: true };

/** What the page may log: ⑨ of 「图片测试」 has none of its files (dev_account.rs), so its thumbnail and its image are
 * not found, as they would not be for a real picture never downloaded. */
const expected = (problem: Problem) => problem.source === 'network' && problem.text.includes('status of 404')
  && /\/api\/chat\/image-data\?session-id=wxid_devpictures&local-id=18&/.test(problem.text);

/** Runs `check` in a new tab: whether it passed. The tab is closed after it, and with it its connections to the server
 * (a tab keeps one open for the live messages, and Chrome opens six at most to a server); so are the tabs the check
 * opened besides. */
async function attempt(browser: Browser, server: { url: string; token: string }, check: Check, shot: string): Promise<boolean> {
  const started = performance.now();
  const address = `${server.url}/ui/?token=${server.token}`;
  const tabs: Page[] = [];
  const open = async () => {
    const page = await browser.newPage();
    tabs.push(page);
    await page.send('Emulation.setDeviceMetricsOverride', check.phone ? PHONE : DESKTOP);
    // what an earlier check chose (the theme, the folded sidebar, the order of the cards) is not kept
    if (tabs.length === 1) await page.send('Storage.clearDataForOrigin', { origin: server.url, storageTypes: 'local_storage' });
    await page.goto(address);
    await page.waitFor('the page to connect to weflow serve', () => document.querySelector('.connection.ready') !== null);
    return page;
  };
  try {
    await check.run(await open(), { address, open });
    const problems = tabs.flatMap(tab => tab.problems).filter(problem => !expected(problem));
    if (problems.length) throw new Error('the page logged errors:\n' + problems.map(p => `    ${p.source}: ${p.text}`).join('\n'));
    console.log(`✓ ${check.name} (${Math.round(performance.now() - started)} ms)`);
    return true;
  } catch (error) {
    console.error(`✗ ${check.name}\n  ${(error as Error).message}`);
    // the tab opened last, in front: where the check was
    const page = tabs.at(-1);
    const waiting = (page?.waiting() ?? []).map(url => '    ' + url.replace(/access_token=[^&]*/, 'access_token=…'));
    if (waiting.length) console.error('  requests still waiting:\n' + waiting.join('\n'));
    // with several tabs, the connections they hold (#878)
    if (tabs.length > 1) {
      const held = await Promise.all(tabs.map(async (tab, i) =>
        `    tab ${i + 1} (${await tab.visibility()}): ${tab.streams().join(', ') || 'no event stream'}`));
      console.error('  the tabs:\n' + held.join('\n'));
    }
    try {
      if (page) {
        await Bun.write(shot, await page.screenshot());
        console.error(`  screenshot: ${shot}`);
      }
    } catch {
      // the page is gone: the message says why
    }
    return false;
  } finally {
    for (const tab of tabs) await tab.close();
  }
}

const { weflow, home } = buildAccount(accounts.dev);
const browser = await Browser.launch(findChrome(), join(out, 'profile'));
let failed = 0, run = 0;
try {
  for (const [lang, all] of [['zh', checks], ['en', englishChecks]] as const) {
    const list = all.filter(check => !only.length || only.some(word => check.name.includes(word)));
    if (!list.length) continue;
    const { serve, url, token } = await startServe(weflow, home, ['--ui-dir', dist, '--no-browser'], { WEFLOW_LANG: lang });
    try {
      for (const check of list) {
        run += 1;
        if (!await attempt(browser, { url, token }, check, join(out, `${lang}-${all.indexOf(check) + 1}.png`))) failed += 1;
      }
    } finally {
      serve.kill();
      await serve.exited;
    }
  }
} finally {
  await browser.close();
}
console.log(failed ? `${failed} of ${run} checks failed` : `all ${run} checks passed`);
process.exit(failed ? 1 : 0);
