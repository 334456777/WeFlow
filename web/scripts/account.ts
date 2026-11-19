// The synthetic accounts `bun run dev` with WEFLOW_DEV_ACCOUNT (#333) and `bun run smoke` (#789) serve: made by an
// example of weflow-cli into target/, and a `weflow serve` of one on a free port of 127.0.0.1 with a token of its own.
// Nothing of a real WeChat account is read.
import { join, resolve } from 'node:path';

const web = join(import.meta.dir, '..');
const root = join(web, '..');
const exe = process.platform === 'win32' ? '.exe' : '';
/** Where cargo builds: `CARGO_TARGET_DIR` when it is set, as for cargo. */
export const target = process.env.CARGO_TARGET_DIR ? resolve(root, process.env.CARGO_TARGET_DIR) : join(root, 'target');

export const accounts = {
  dev: { example: 'dev_account', dir: 'dev-account', profile: 'debug' },
  // a debug build would take many minutes to draw and encrypt that many messages
  bench: { example: 'bench_account', dir: 'bench-account', profile: 'release' },
} as const;
export type Account = (typeof accounts)[keyof typeof accounts];

function cargo(args: string[]): boolean {
  try {
    return Bun.spawnSync(['cargo', 'build', ...args], { cwd: root, stdio: ['inherit', 'inherit', 'inherit'] }).exitCode === 0;
  } catch (error) {
    console.error(`cargo could not start (${(error as Error).message}): https://rustup.rs`);
    process.exit(1);
  }
}

/** `weflow` and the fake model API (crates/weflow-fake-model), in a debug build. */
const binaries = ['-p', 'weflow-cli', '-p', 'weflow-fake-model', '--bin', 'weflow', '--bin', 'weflow-fake-model'];

/** Builds `weflow`, the fake model API and the account's generator, which makes the account when it is not there yet:
 * the binary and the WeFlow home that points at the account. Exits when a step fails. */
export function buildAccount(chosen: Account): { weflow: string; home: string } {
  // debug build: what `cargo build` has made already, and quick to make again after a change
  // one cargo run for the debug account; the benchmark account's generator is built in release, so it is a run of its own
  const builds = chosen.profile === 'release'
    ? [binaries, ['-p', 'weflow-cli', '--example', chosen.example, '--release']]
    : [[...binaries, '--example', chosen.example]];
  if (!builds.every(cargo)) {
    console.error('cargo build failed');
    process.exit(1);
  }
  const weflow = join(target, 'debug/weflow' + exe);
  const generator = join(target, `${chosen.profile}/examples/${chosen.example}${exe}`);
  const account = Bun.spawnSync([generator, join(target, chosen.dir), '--weflow', weflow], {
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  if (account.exitCode !== 0) process.exit(1);
  return { weflow, home: account.stdout.toString().trim() };
}

/** `weflow serve` of the WeFlow home `home` with `args` (and `env` added to its environment), on a free port with a
 * random token, once it has started. */
export async function startServe(weflow: string, home: string, args: string[], env: Record<string, string> = {}) {
  const probe = Bun.listen({ hostname: '127.0.0.1', port: 0, socket: { data() {} } });
  const port = probe.port;
  probe.stop(true);
  const token = crypto.getRandomValues(new Uint8Array(16)).toHex();
  const serve = Bun.spawn([weflow, '--json', 'serve', ...args, '--port', String(port)], {
    env: { ...process.env, ...env, WEFLOW_HOME: home, WEFLOW_HTTP_TOKEN: token }, stdio: ['ignore', 'ignore', 'pipe'],
  });
  // stderr is read to the end so that the server never waits on a full pipe; its errors are passed on
  const started = Promise.withResolvers<void>();
  serve.exited.then(code => started.reject(new Error(`weflow serve stopped (${code}) before it started`)));
  (async () => {
    let rest = '';
    for await (const chunk of serve.stderr.pipeThrough(new TextDecoderStream())) {
      const lines = (rest + chunk).split(/\r?\n/);
      rest = lines.pop()!;
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
  return { serve, url: `http://127.0.0.1:${port}`, token };
}

/** The fake model API on a free port of 127.0.0.1, once it has started: its process and its addresses. */
export async function startFakeModel() {
  const fake = Bun.spawn([join(target, 'debug/weflow-fake-model' + exe)], { stdio: ['ignore', 'pipe', 'inherit'] });
  // it prints one line with its addresses, and nothing after
  const reader = fake.stdout.pipeThrough(new TextDecoderStream()).getReader();
  let line = '';
  while (!line.includes('\n')) {
    const { value, done } = await reader.read();
    if (done) throw new Error('weflow-fake-model stopped before it started');
    line += value;
  }
  reader.releaseLock();
  const started: { openaiBaseUrl: string; anthropicBaseUrl: string; key: string } = JSON.parse(line);
  return { fake, ...started };
}

/** Points the AI settings of the WeFlow home `home` at the fake model API (OpenAI's format, `fake-chat`), unless they
 * name a service other than an earlier fake's: its port changes from run to run. */
export function useFakeModel(weflow: string, home: string, fake: { openaiBaseUrl: string; key: string }) {
  const config = (...args: string[]) => Bun.spawnSync([weflow, '--json', 'config', ...args], {
    env: { ...process.env, WEFLOW_HOME: home }, stdio: ['ignore', 'pipe', 'pipe'],
  });
  const read = (key: string): unknown => {
    try { return JSON.parse(config('get', key).stdout.toString()).data; } catch { return null; }
  };
  const base = read('ai_model_api_base_url');
  if (typeof base === 'string' && base.trim() && !base.startsWith('http://127.0.0.1:')) return false;
  const settings: [string, string][] = [['ai_model_api_base_url', fake.openaiBaseUrl], ['ai_model_api_key', fake.key]];
  // the format and the model the developer chose are kept
  if (!read('ai_model_api_model')) settings.push(['ai_model_api_model', 'fake-chat']);
  // the same address serves Anthropic's format: after a base that ends in /v1, weflow asks /v1/messages
  for (const [key, value] of settings) config('set', key, value);
  return true;
}
