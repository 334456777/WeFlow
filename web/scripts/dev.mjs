// `bun run dev` (#333): the page Vite serves from these sources, the same as `bun run build` makes it, over a
// `weflow serve` of a synthetic account (crates/weflow-cli/examples/dev_account.rs) on a free port with a token of
// its own, which the proxy adds. Nothing of a real WeChat account is read. With WEFLOW_SERVE_URL set (in the shell or
// web/.env.local) the proxy goes to that server instead, with WEFLOW_HTTP_TOKEN, and nothing else is started.
// WEFLOW_DEV_ACCOUNT=bench (likewise) serves the benchmark account instead (crates/weflow-cli/examples/bench_account.rs,
// #488): as big and as varied as a real one, but its JPEGs are not pictures; the first build takes a minute or two and
// about 2.5 GB in target/bench-account.
// Arguments go on to Vite: `bun run dev --host`. Runs on Bun only (#360), and Vite with it.
import { join } from 'node:path';
import { loadEnv } from 'vite';
import { accounts, buildAccount, startServe } from './account.ts';

const web = join(import.meta.dir, '..');

function vite(env) {
  return Bun.spawn([process.execPath, join(web, 'node_modules/vite/bin/vite.js'), ...process.argv.slice(2)], {
    cwd: web, stdio: ['inherit', 'inherit', 'inherit'], env: { ...process.env, ...env },
  });
}

const env = loadEnv('development', web, 'WEFLOW_');
const chosen = accounts[env.WEFLOW_DEV_ACCOUNT || 'dev'];
if (!chosen) {
  console.error(`WEFLOW_DEV_ACCOUNT=${env.WEFLOW_DEV_ACCOUNT}: use one of ${Object.keys(accounts).join(', ')}`);
  process.exit(1);
}

if (env.WEFLOW_SERVE_URL) {
  const page = vite({});
  // a kill of this script alone (not Ctrl-C, which reaches both) stops Vite too
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => page.kill(signal));
  process.exit(await page.exited);
} else {
  const { weflow, home } = buildAccount(chosen);
  const { serve, url, token } = await startServe(weflow, home, ['--api-only']).catch(error => {
    console.error(error.message);
    process.exit(1);
  });
  let page = null;
  const stop = code => {
    page?.kill();
    serve.kill();
    process.exit(code);
  };
  serve.exited.then(code => {
    console.error(`weflow serve stopped (${code}); the page has no server now`);
    stop(code || 1);
  });
  console.log(`weflow serve: the synthetic account (${chosen.example}) on ${url}`);
  page = vite({ WEFLOW_SERVE_URL: url, WEFLOW_HTTP_TOKEN: token });
  page.exited.then(code => stop(code ?? 0));
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => stop(0));
}
