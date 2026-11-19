// `bun run dev` (#333): the page Vite serves from these sources, the same as `bun run build` makes it. By itself it
// starts Vite alone, with no server behind it: the page answers itself from the sample server (src/mock, #879).
// With WEFLOW_SERVE_URL set (in the shell or web/.env.local) the proxy goes to that server, with WEFLOW_HTTP_TOKEN, and
// nothing else is started.
// WEFLOW_DEV_ACCOUNT (likewise) starts a `weflow serve` of a synthetic account on a free port with a token of its own,
// which the proxy adds: `dev` (crates/weflow-cli/examples/dev_account.rs) or `bench`, the benchmark account
// (crates/weflow-cli/examples/bench_account.rs, #488), as big and as varied as a real one, but its JPEGs are not
// pictures; its first build takes a minute or two and about 2.5 GB in target/bench-account. Nothing of a real WeChat
// account is read. The account's AI settings point at the fake model API (crates/weflow-fake-model), started beside
// it, unless they name another service.
// Arguments go on to Vite: `bun run dev --host`. Runs on Bun only (#360), and Vite with it.
import { join } from 'node:path';
import { loadEnv } from 'vite';
import { accounts, buildAccount, startFakeModel, startServe, useFakeModel } from './account.ts';

const web = join(import.meta.dir, '..');

function vite(env) {
  return Bun.spawn([process.execPath, join(web, 'node_modules/vite/bin/vite.js'), ...process.argv.slice(2)], {
    cwd: web, stdio: ['inherit', 'inherit', 'inherit'], env: { ...process.env, ...env },
  });
}

const env = loadEnv('development', web, 'WEFLOW_');
const chosen = Object.hasOwn(accounts, env.WEFLOW_DEV_ACCOUNT ?? '') ? accounts[env.WEFLOW_DEV_ACCOUNT] : null;
if (env.WEFLOW_DEV_ACCOUNT && !chosen) {
  console.error(`WEFLOW_DEV_ACCOUNT=${env.WEFLOW_DEV_ACCOUNT}: use one of ${Object.keys(accounts).join(', ')}`);
  process.exit(1);
}

if (env.WEFLOW_SERVE_URL || !chosen) {
  if (!env.WEFLOW_SERVE_URL) console.log('no weflow serve: the page shows the sample records (WEFLOW_DEV_ACCOUNT=dev serves the synthetic account)');
  const page = vite({});
  // a kill of this script alone (not Ctrl-C, which reaches both) stops Vite too
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => page.kill(signal));
  process.exit(await page.exited);
} else {
  const { weflow, home } = buildAccount(chosen);
  const model = await startFakeModel().catch(error => {
    console.error(error.message);
    process.exit(1);
  });
  if (useFakeModel(weflow, home, model)) console.log(`fake model API: ${model.openaiBaseUrl} (OpenAI), ${model.anthropicBaseUrl} (Anthropic)`);
  const { serve, url, token } = await startServe(weflow, home, ['--api-only']).catch(error => {
    model.fake.kill();
    console.error(error.message);
    process.exit(1);
  });
  let page = null;
  const stop = code => {
    page?.kill();
    serve.kill();
    model.fake.kill();
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
