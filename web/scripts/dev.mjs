// `npm run dev` (#333): the page Vite serves from these sources, the same as `npm run build` makes it, over a
// `weflow serve` of a synthetic account (crates/weflow-cli/examples/dev_account.rs) on a free port with a token of
// its own, which the proxy adds. Nothing of a real WeChat account is read. With WEFLOW_SERVE_URL set (in the shell or
// web/.env.local) the proxy goes to that server instead, with WEFLOW_HTTP_TOKEN, and nothing else is started.
// Arguments go on to Vite: `npm run dev -- --host`.
import { spawn, spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { createServer } from 'node:net';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { loadEnv } from 'vite';

const web = fileURLToPath(new URL('..', import.meta.url));
const root = join(web, '..');
const exe = process.platform === 'win32' ? '.exe' : '';

function vite(env) {
  const child = spawn(process.execPath, [join(web, 'node_modules/vite/bin/vite.js'), ...process.argv.slice(2)], {
    cwd: web, stdio: 'inherit', env: { ...process.env, ...env },
  });
  return child;
}

if (loadEnv('development', web, 'WEFLOW_').WEFLOW_SERVE_URL) {
  vite({}).on('exit', code => process.exit(code ?? 0));
} else {
  // debug build: what `cargo build` has made already, and quick to make again after a change
  const built = spawnSync('cargo', ['build', '-p', 'weflow-cli', '--bin', 'weflow', '--example', 'dev_account'], { cwd: root, stdio: 'inherit' });
  if (built.status !== 0) {
    console.error(built.error ? `cargo could not start (${built.error.message}): https://rustup.rs` : 'cargo build failed');
    process.exit(1);
  }
  const weflow = join(root, 'target/debug/weflow' + exe);
  const account = spawnSync(join(root, 'target/debug/examples/dev_account' + exe), [join(root, 'target/dev-account'), '--weflow', weflow], {
    stdio: ['ignore', 'pipe', 'inherit'], encoding: 'utf8',
  });
  if (account.status !== 0) process.exit(1);
  const home = account.stdout.trim();

  const port = await new Promise((resolve, reject) => {
    const probe = createServer().once('error', reject).listen(0, '127.0.0.1', () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });
  const token = randomBytes(16).toString('hex');
  const serve = spawn(weflow, ['--json', 'serve', '--api-only', '--port', String(port)], {
    env: { ...process.env, WEFLOW_HOME: home, WEFLOW_HTTP_TOKEN: token }, stdio: ['ignore', 'ignore', 'pipe'],
  });
  let page = null;
  const stop = code => {
    page?.kill();
    serve.kill();
    process.exit(code);
  };
  serve.on('exit', code => {
    if (page) console.error(`weflow serve stopped (${code}); the page has no server now`);
    stop(code || 1);
  });
  // stderr is read to the end so that the server never waits on a full pipe; its errors are passed on
  await new Promise(resolve => createInterface({ input: serve.stderr }).on('line', line => {
    try {
      const event = JSON.parse(line);
      if (event.type === 'server_started') resolve();
      else if (event.level === 'error' || event.type === 'error') console.error('weflow serve:', event.message ?? line);
    } catch {
      console.error('weflow serve:', line);
    }
  }));
  console.log(`weflow serve: the synthetic account on http://127.0.0.1:${port}`);
  page = vite({ WEFLOW_SERVE_URL: `http://127.0.0.1:${port}`, WEFLOW_HTTP_TOKEN: token });
  page.on('exit', code => stop(code ?? 0));
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => stop(0));
}
