// Runs Claude Code through the Agent SDK on one prompt, read-only: the only tools are Read, Grep and Glob, nothing is
// pre-approved, and anything that would ask for permission (reading outside the working directory) is denied.

import { query, type SDKResultMessage } from "@anthropic-ai/claude-agent-sdk";
import type { Provider } from "./providers.ts";

const TOOLS = ["Read", "Grep", "Glob"];

/** A finished answer. */
export interface Answer {
  /** The final message. */
  text: string;
  /** The model that wrote it, as Claude Code reports it. */
  model: string;
  /** The cost Claude Code computed, in US dollars. */
  costUsd: number;
}

/** A run that ended without an answer. */
export class AnswerError extends Error {
  /** Whether the model did any work (and was paid for it) before the run ended. */
  readonly started: boolean;

  constructor(message: string, started: boolean) {
    super(message);
    this.started = started;
  }
}

function failure(result: SDKResultMessage): string {
  if (result.subtype === "success") {
    return `HTTP ${result.api_error_status ?? "?"}: ${result.result}`;
  }
  return [result.subtype, ...result.errors].join(": ");
}

/** What a run may do. */
export interface Limits {
  /** The model or model alias. */
  model: string;
  /** The most turns before the run stops. */
  maxTurns: number;
  /** The most the run may cost, in US dollars. */
  maxBudgetUsd: number;
}

/** Answers the prompt in `cwd` through the provider. */
export async function answer(
  prompt: string,
  append: string,
  cwd: string,
  provider: Provider,
  limits: Limits,
): Promise<Answer> {
  let model = "";
  let result: SDKResultMessage | undefined;
  try {
    for await (const message of query({
      prompt,
      options: {
        cwd,
        env: provider.env,
        model: limits.model,
        maxTurns: limits.maxTurns,
        maxBudgetUsd: limits.maxBudgetUsd,
        tools: TOOLS,
        permissionMode: "dontAsk",
        // CLAUDE.md of the checkout; there are no settings files on the runner or in the repository
        settingSources: ["project"],
        skills: [],
        mcpServers: {},
        strictMcpConfig: true,
        persistSession: false,
        systemPrompt: { type: "preset", preset: "claude_code", append },
        stderr: (data) => process.stderr.write(data),
      },
    })) {
      if (message.type === "system" && message.subtype === "init") model = message.model;
      if (message.type === "result") result = message;
    }
  } catch (error) {
    throw new AnswerError(`${provider.name}: ${error}`, (result?.total_cost_usd ?? 0) > 0);
  }
  if (result === undefined) throw new AnswerError(`${provider.name}: no result`, false);
  for (const denial of result.permission_denials) {
    console.log(`denied ${denial.tool_name} ${JSON.stringify(denial.tool_input)}`);
  }
  if (result.subtype !== "success" || result.is_error) {
    throw new AnswerError(`${provider.name}: ${failure(result)}`, result.total_cost_usd > 0);
  }
  return { text: result.result, model, costUsd: result.total_cost_usd };
}
