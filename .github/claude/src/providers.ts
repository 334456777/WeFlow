// The model providers Claude Code can answer through, each as the environment of the Claude Code process: the
// Claude API signed in by Workload Identity Federation, and DeepSeek's Anthropic-compatible API with the variables its
// guide gives (https://api-docs.deepseek.com/zh-cn/guides/anthropic_api/). The environment replaces the job's, so
// Claude Code gets no GitHub token and no way to ask for OIDC tokens.

import { mkdir, rename, writeFile } from "node:fs/promises";
import { join } from "node:path";

/** Renew the GitHub OIDC token this often: it expires five minutes after it is issued. */
const RENEW_MS = 4 * 60 * 1000;

/** A provider and the environment that points Claude Code at it. */
export interface Provider {
  /** A name for logs. */
  name: string;
  /** The environment of the Claude Code process. */
  env: Record<string, string>;
  /** Stops what the provider runs in the background. */
  stop(): void;
}

function baseEnv(): Record<string, string> {
  const env: Record<string, string> = {
    LANG: "C.UTF-8",
    CLAUDE_AGENT_SDK_CLIENT_APP: "weflow-claude",
  };
  for (const name of ["PATH", "HOME", "TMPDIR"]) {
    const value = process.env[name];
    if (value) env[name] = value;
  }
  return env;
}

async function fetchIdToken(requestUrl: string, requestToken: string): Promise<string> {
  const url = new URL(requestUrl);
  url.searchParams.set("audience", "https://api.anthropic.com");
  const response = await fetch(url, { headers: { Authorization: `Bearer ${requestToken}` } });
  if (!response.ok) throw new Error(`requesting the GitHub OIDC token: HTTP ${response.status}`);
  const { value } = (await response.json()) as { value: string };
  console.log(`::add-mask::${value}`);
  return value;
}

/**
 * The Claude API, signed in by Workload Identity Federation: Claude Code exchanges the GitHub OIDC token in a file
 * for an access token, and reads the file again when the access token runs out, so the file is renewed in the
 * background. Undefined without the federation's identifiers or the job's `id-token: write`.
 */
export async function anthropic(): Promise<Provider | undefined> {
  const {
    ANTHROPIC_FEDERATION_RULE_ID: rule,
    ANTHROPIC_ORGANIZATION_ID: organization,
    ANTHROPIC_SERVICE_ACCOUNT_ID: serviceAccount,
    ANTHROPIC_WORKSPACE_ID: workspace,
    ACTIONS_ID_TOKEN_REQUEST_URL: requestUrl,
    ACTIONS_ID_TOKEN_REQUEST_TOKEN: requestToken,
    RUNNER_TEMP: temp,
  } = process.env;
  if (!rule || !organization || !serviceAccount || !workspace || !requestUrl || !requestToken || !temp) {
    return undefined;
  }
  const dir = join(temp, "claude-oidc");
  const file = join(dir, "identity-token");
  await mkdir(dir, { recursive: true });
  const renew = async () => {
    await writeFile(`${file}.tmp`, await fetchIdToken(requestUrl, requestToken), { mode: 0o600 });
    await rename(`${file}.tmp`, file);
  };
  await renew();
  const timer = setInterval(() => {
    renew().catch((error) => console.log(`::warning::${error}`));
  }, RENEW_MS);
  return {
    name: "Claude API",
    env: {
      ...baseEnv(),
      ANTHROPIC_FEDERATION_RULE_ID: rule,
      ANTHROPIC_ORGANIZATION_ID: organization,
      ANTHROPIC_SERVICE_ACCOUNT_ID: serviceAccount,
      ANTHROPIC_WORKSPACE_ID: workspace,
      ANTHROPIC_IDENTITY_TOKEN_FILE: file,
    },
    stop: () => clearInterval(timer),
  };
}

/** DeepSeek's Anthropic-compatible API, set up as its guide sets up Claude Code. Undefined without a key. */
export function deepseek(): Provider | undefined {
  const key = process.env.DEEPSEEK_API_KEY;
  if (!key) return undefined;
  return {
    name: "DeepSeek",
    env: {
      ...baseEnv(),
      ANTHROPIC_BASE_URL: "https://api.deepseek.com/anthropic",
      ANTHROPIC_AUTH_TOKEN: key,
      ANTHROPIC_MODEL: "deepseek-flash[1m]",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "deepseek-flash[1m]",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "deepseek-flash[1m]",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "deepseek-flash",
      CLAUDE_CODE_SUBAGENT_MODEL: "deepseek-flash",
      CLAUDE_CODE_EFFORT_LEVEL: "max",
      CLAUDE_CODE_AUTO_COMPACT_WINDOW: "786432",
    },
    stop: () => {},
  };
}
