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

const web = join(import.meta.dir, '..');
const root = join(web, '..');
const exe = process.platform === 'win32' ? '.exe' : '';

function vite(env) {
  return Bun.spawn([process.execPath, join(web, 'node_modules/vite/bin/vite.js'), ...process.argv.slice(2)], {
    cwd: web, stdio: ['inherit', 'inherit', 'inherit'], env: { ...process.env, ...env },
  });
}

const env = loadEnv('development', web, 'WEFLOW_');
const accounts = {
  dev: { example: 'dev_account', dir: 'target/dev-account', profile: 'debug' },
  // a debug build would take many minutes to draw and encrypt that many messages
  bench: { example: 'bench_account', dir: 'target/bench-account', profile: 'release' },
};
const chosen = accounts[env.WEFLOW_DEV_ACCOUNT || 'dev'];
if (!chosen) {
  console.error(`WEFLOW_DEV_ACCOUNT=${env.WEFLOW_DEV_ACCOUNT}: use one of ${Object.keys(accounts).join(', ')}`);
  process.exit(1);
}

function cargo(args) {
  try {
    return Bun.spawnSync(['cargo', 'build', ...args], { cwd: root, stdio: ['inherit', 'inherit', 'inherit'] }).exitCode === 0;
  } catch (error) {
    console.error(`cargo could not start (${error.message}): https://rustup.rs`);
    process.exit(1);
  }
}

if (env.WEFLOW_SERVE_URL) {
  const page = vite({});
  // a kill of this script alone (not Ctrl-C, which reaches both) stops Vite too
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => page.kill(signal));
  process.exit(await page.exited);
} else {
  // debug build: what `cargo build` has made already, and quick to make again after a change
  // one cargo run for the debug account; the benchmark account's generator is built in release, so it is a run of its own
  const builds = chosen.profile === 'release'
    ? [['-p', 'weflow-cli', '--bin', 'weflow'], ['-p', 'weflow-cli', '--example', chosen.example, '--release']]
    : [['-p', 'weflow-cli', '--bin', 'weflow', '--example', chosen.example]];
  if (!builds.every(cargo)) {
    console.error('cargo build failed');
    process.exit(1);
  }
  const weflow = join(root, 'target/debug/weflow' + exe);
  const generator = join(root, `target/${chosen.profile}/examples/${chosen.example}${exe}`);
  const account = Bun.spawnSync([generator, join(root, chosen.dir), '--weflow', weflow], {
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  if (account.exitCode !== 0) process.exit(1);
  const home = account.stdout.toString().trim();

  const probe = Bun.listen({ hostname: '127.0.0.1', port: 0, socket: { data() {} } });
  const port = probe.port;
  probe.stop(true);
  const token = crypto.getRandomValues(new Uint8Array(16)).toHex();
  const serve = Bun.spawn([weflow, '--json', 'serve', '--api-only', '--port', String(port)], {
    env: { ...process.env, WEFLOW_HOME: home, WEFLOW_HTTP_TOKEN: token }, stdio: ['ignore', 'ignore', 'pipe'],
  });
  let page = null;
  const stop = code => {
    page?.kill();
    serve.kill();
    process.exit(code);
  };
  serve.exited.then(code => {
    if (page) console.error(`weflow serve stopped (${code}); the page has no server now`);
    stop(code || 1);
  });
  // stderr is read to the end so that the server never waits on a full pipe; its errors are passed on
  const started = Promise.withResolvers();
  (async () => {
    let rest = '';
    for await (const chunk of serve.stderr.pipeThrough(new TextDecoderStream())) {
      const lines = (rest + chunk).split(/\r?\n/);
      rest = lines.pop();
      for (const line of lines) {
        if (!line) continue;
        try {
          const event = JSON.parse(line);
          if (event.type === 'server_started') started.resolve();
          else if (event.level === 'error' || event.type === 'error') console.error('weflow serve:', event.message ?? line);
        } catch {
          console.error('weflow serve:', line);
        }
      }
    }
  })();
  await started.promise;
  console.log(`weflow serve: the synthetic account (${chosen.example}) on http://127.0.0.1:${port}`);
  page = vite({ WEFLOW_SERVE_URL: `http://127.0.0.1:${port}`, WEFLOW_HTTP_TOKEN: token });
  page.exited.then(code => stop(code ?? 0));
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => stop(0));
}
