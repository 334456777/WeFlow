// The synthetic accounts `bun run dev` serves (#333): made by an example of weflow-cli into target/, and a
// `weflow serve` of one on a free port of 127.0.0.1 with a token of its own. Nothing of a real WeChat account is read.
import { join } from 'node:path';

const web = join(import.meta.dir, '..');
const root = join(web, '..');
const exe = process.platform === 'win32' ? '.exe' : '';

export const accounts = {
  dev: { example: 'dev_account', dir: 'target/dev-account', profile: 'debug' },
  // a debug build would take many minutes to draw and encrypt that many messages
  bench: { example: 'bench_account', dir: 'target/bench-account', profile: 'release' },
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

/** Builds `weflow` and the account's generator, which makes the account when it is not there yet: the binary and the
 * WeFlow home that points at the account. Exits when a step fails. */
export function buildAccount(chosen: Account): { weflow: string; home: string } {
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
  return { weflow, home: account.stdout.toString().trim() };
}

/** `weflow serve` of the WeFlow home `home` with `args`, on a free port with a random token, once it has started. */
export async function startServe(weflow: string, home: string, args: string[]) {
  const probe = Bun.listen({ hostname: '127.0.0.1', port: 0, socket: { data() {} } });
  const port = probe.port;
  probe.stop(true);
  const token = crypto.getRandomValues(new Uint8Array(16)).toHex();
  const serve = Bun.spawn([weflow, '--json', 'serve', ...args, '--port', String(port)], {
    env: { ...process.env, WEFLOW_HOME: home, WEFLOW_HTTP_TOKEN: token }, stdio: ['ignore', 'ignore', 'pipe'],
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
