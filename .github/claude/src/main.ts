// Answers a mention of @claude in an issue or a pull request (claude.yml): checks that the user who wrote it has
// write access, acknowledges it with a reaction, asks Claude Code through the Agent SDK with the issue or pull request
// as the prompt, and posts the answer as github-actions[bot] (in the thread, for a review comment). The Claude API is
// tried first; when it ends before doing any work (a billing or sign-in error) and a DeepSeek key is set, DeepSeek
// answers instead.
//
// usage: bun src/main.ts [<event name> <event payload file>]   (default: the workflow's event)
// env: GITHUB_TOKEN (issues: write, pull-requests: write), GITHUB_REPOSITORY, GITHUB_WORKSPACE (the checkout of the
//      default branch), GITHUB_RUN_ID; the providers' variables (providers.ts); DRY_RUN=1 prints the answer instead
//      of posting it and adds no reaction.

import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { answer, AnswerError, type Answer, type Limits } from "./answer.ts";
import { readMention, type GitHubEvent } from "./event.ts";
import { GitHub } from "./github.ts";
import { systemAppend, userPrompt } from "./prompt.ts";
import { anthropic, deepseek, type Provider } from "./providers.ts";
import { describeThread } from "./thread.ts";

const LIMITS: Limits = { model: "opus", maxTurns: 40, maxBudgetUsd: 5 };

function required(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is not set`);
  return value;
}

async function answerThroughFirst(providers: Provider[], prompt: string, append: string, cwd: string): Promise<Answer> {
  if (providers.length === 0) {
    throw new Error("no provider is set up: give the federation's identifiers and id-token: write, or DEEPSEEK_API_KEY");
  }
  for (const [i, provider] of providers.entries()) {
    try {
      return await answer(prompt, append, cwd, provider, LIMITS);
    } catch (error) {
      const next = providers[i + 1];
      if (next === undefined || !(error instanceof AnswerError) || error.started) throw error;
      console.log(`::warning::${error.message}; answering through ${next.name}`);
    }
  }
  throw new Error("unreachable");
}

async function main(): Promise<number> {
  const [eventName = required("GITHUB_EVENT_NAME"), eventPath = required("GITHUB_EVENT_PATH")] = process.argv.slice(2);
  const repository = required("GITHUB_REPOSITORY");
  const cwd = required("GITHUB_WORKSPACE");
  const serverUrl = process.env.GITHUB_SERVER_URL ?? "https://github.com";
  const runUrl = `${serverUrl}/${repository}/actions/runs/${process.env.GITHUB_RUN_ID}`;
  const dryRun = process.env.DRY_RUN === "1";
  const github = new GitHub(repository, required("GITHUB_TOKEN"), process.env.GITHUB_API_URL);

  const mention = readMention(eventName, JSON.parse(await readFile(eventPath, "utf8")) as GitHubEvent);
  const permission = await github.permission(mention.asker);
  if (permission !== "admin" && permission !== "write") {
    console.log(`@${mention.asker} has ${permission} access; only users with write access are answered.`);
    return 0;
  }
  if (!dryRun && mention.reactionsPath !== undefined) {
    await github.post(mention.reactionsPath, { content: "eyes" }).catch((error) => console.log(`::warning::${error}`));
  }

  const providers = [await anthropic(), deepseek()].filter((p): p is Provider => p !== undefined);
  let body: string;
  let status = 0;
  try {
    const sha = execFileSync("git", ["rev-parse", "HEAD"], { cwd, encoding: "utf8" }).trim();
    const thread = await describeThread(github, mention);
    const append = systemAppend(mention, { repository, sha, serverUrl });
    const result = await answerThroughFirst(providers, userPrompt(mention, thread), append, cwd);
    console.log(`${result.model}: US$${result.costUsd.toFixed(4)}`);
    body = `${result.text}\n\n<sub>${result.model} · [运行记录](${runUrl})</sub>`;
  } catch (error) {
    console.log(`::error::${error instanceof Error ? error.message : error}`);
    body = `没能回答这次 @claude，原因见[运行记录](${runUrl})。`;
    status = 1;
  } finally {
    for (const provider of providers) provider.stop();
  }
  if (dryRun) console.log(body);
  else await github.post(mention.replyPath, { body });
  return status;
}

process.exitCode = await main();
