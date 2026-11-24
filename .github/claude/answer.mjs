// Answers a mention of @claude in an issue or a pull request (.github/workflows/claude.yml) with the Claude Agent SDK:
// 👀 on what asked, Claude Code reads the repository and the thread, and its answer is posted in the same place,
// ending with the model and a link to this run. The endpoint, model and effort come from the repository's Actions
// variables (see applyConfig). Tool calls outside the read-only rules are denied and logged as warnings. A run that
// can't answer replies 没有回答 and fails.
//
// The script installs the SDK itself (`npm ci`), while it reacts and reads the thread: the Claude Code binary the
// SDK brings is a few hundred MB, and a failed install still gets its reply.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

import { canWrite, checkoutSha, fetchPrHead, react, readAsk, readThread, reply, runUrl } from './github.mjs';
import { instructions, prompt } from './prompt.mjs';

const HERE = fileURLToPath(new URL('.', import.meta.url));
// The job's timeout-minutes is 30: giving up earlier leaves time to reply that it failed.
const TIMEOUT_MINUTES = 25;
// GitHub refuses comments longer than 65536 characters.
const MAX_REPLY = 60_000;

// The tools Claude gets: reading and searching, the shell, the web, subagents and the repository's skills; none
// that edit files.
const TOOLS = ['Read', 'Grep', 'Glob', 'Bash', 'WebFetch', 'WebSearch', 'Agent', 'Skill', 'TodoWrite'];
// Commands that run without asking, besides reads in the working directory and Claude Code's built-in read-only
// commands (ls, cat, grep, find, read-only git, …), which need no rule. `gh api` isn't here: it can write.
const GH_READS = ['issue view', 'issue list', 'pr view', 'pr diff', 'pr checks', 'pr list', 'run view', 'run list'];
const ALLOWED = [...GH_READS, 'search', 'release view', 'release list'].map((command) => `Bash(gh ${command} *)`);
// Tools whose every call canUseTool allows: none of them writes. Every other call that would ask is denied.
const OPEN = new Set(['WebFetch', 'WebSearch', 'Agent', 'Skill', 'TodoWrite']);

/**
 * Turns the repository's Actions variables, which claude.yml passes as JSON in CLAUDE_VARS, into environment variables
 * of the same name for Claude Code (ANTHROPIC_BASE_URL, ANTHROPIC_MODEL, CLAUDE_CODE_EFFORT_LEVEL, …), so that they
 * change without editing a file. A variable doesn't override what the workflow or the runner set. The API key secrets
 * the repository doesn't have arrive empty and are dropped: Claude Code would take an empty value as set.
 */
function applyConfig(ask) {
  for (const name of ['ANTHROPIC_API_KEY', 'ANTHROPIC_AUTH_TOKEN']) {
    if (!process.env[name]) delete process.env[name];
  }
  const vars = JSON.parse(process.env.CLAUDE_VARS || '{}');
  delete process.env.CLAUDE_VARS;
  for (const [name, value] of Object.entries(vars)) {
    if (!(name in process.env)) process.env[name] = value;
  }
  if (ask.model) process.env.ANTHROPIC_MODEL = ask.model;
}

/** A run that ends without an answer; `reason` goes into the reply. */
class Unanswered extends Error {
  constructor(reason, detail) {
    super(`${reason}: ${detail}`);
    this.reason = reason;
  }
}

// Workflow commands (::warning:: and the like) need %, CR and LF escaped.
const escape = (text) => text.replaceAll('%', '%25').replaceAll('\r', '%0D').replaceAll('\n', '%0A');
const clip = (text, length) => (text.length > length ? `${text.slice(0, length)}…` : text);

function install() {
  return new Promise((resolve, reject) => {
    // one string through the shell, which also finds npm.cmd on Windows
    const npm = spawn('npm ci --ignore-scripts --loglevel=error', { cwd: HERE, stdio: 'inherit', shell: true });
    npm.on('error', reject);
    npm.on('exit', (code) =>
      code === 0 ? resolve() : reject(new Unanswered('安装 Agent SDK 失败', `npm ci exited with ${code}`)),
    );
  });
}

const denied = new Set();

function logDenied(tool, input, id) {
  if (denied.has(id)) return;
  denied.add(id);
  console.log(`::warning title=被拒绝的工具调用::${escape(`${tool} ${clip(JSON.stringify(input), 1000)}`)}`);
}

function logAssistant(message) {
  const indent = message.parent_tool_use_id ? '    ' : '';
  for (const block of message.message.content) {
    if (block.type === 'text' && block.text.trim()) console.log(`${indent}${clip(block.text.trim(), 2000)}`);
    if (block.type === 'tool_use') console.log(`${indent}→ ${block.name} ${clip(JSON.stringify(block.input), 300)}`);
  }
}

/** Runs Claude Code on the thread; returns the answer and the model that wrote it. */
async function answer(query, ask, thread) {
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), TIMEOUT_MINUTES * 60_000);
  let model = process.env.ANTHROPIC_MODEL ?? '';
  let result;
  try {
    const messages = query({
      prompt: prompt(ask, thread),
      options: {
        cwd: process.env.GITHUB_WORKSPACE,
        // CLAUDE.md and the skills of the default branch
        settingSources: ['project'],
        systemPrompt: { type: 'preset', preset: 'claude_code', append: instructions(checkoutSha(), thread.pr) },
        tools: TOOLS,
        allowedTools: ALLOWED,
        // called for every call that would ask for permission, as nobody is there to answer
        canUseTool: async (tool, input, { toolUseID }) => {
          if (OPEN.has(tool)) return { behavior: 'allow', updatedInput: input };
          logDenied(tool, input, toolUseID);
          return {
            behavior: 'deny',
            message: `这次运行只能读，这个 ${tool} 调用不在允许的范围内。换只读的办法，或在回答里写明需要维护者做什么。`,
          };
        },
        abortController: abort,
        stderr: (data) => process.stderr.write(data),
      },
    });
    for await (const message of messages) {
      if (message.type === 'system' && message.subtype === 'init') {
        model = message.model;
        const effort = message.effort ?? process.env.CLAUDE_CODE_EFFORT_LEVEL ?? 'default';
        console.log(`Claude Code ${message.claude_code_version}, model ${message.model}, effort ${effort}`);
      } else if (message.type === 'assistant') {
        logAssistant(message);
      } else if (message.type === 'result') {
        result = message;
      }
    }
  } catch (err) {
    // the SDK throws after an error result, which says more than the exception
    if (!abort.signal.aborted && !result) throw err;
    console.log(`::warning::${escape(err.message)}`);
  } finally {
    clearTimeout(timer);
  }
  if (abort.signal.aborted) throw new Unanswered(`超过 ${TIMEOUT_MINUTES} 分钟`, 'timed out');
  if (!result) throw new Unanswered('运行出错', 'Claude Code ended without a result');
  // the authoritative list, which also has the calls denied by a rule rather than by canUseTool
  for (const d of result.permission_denials ?? []) logDenied(d.tool_name, d.tool_input, d.tool_use_id);
  console.log(
    `${result.num_turns} turns in ${Math.round(result.duration_ms / 1000)} s; models ${Object.keys(result.modelUsage).join(', ')}`,
  );
  if (result.subtype !== 'success') {
    const reason = result.subtype === 'error_max_turns' ? '对话轮数到了上限' : '运行出错';
    throw new Unanswered(reason, `${result.subtype} ${(result.errors ?? []).join('; ')}`);
  }
  if (result.is_error) {
    const reason = result.api_error_status ? `模型接口返回 HTTP ${result.api_error_status}` : '运行出错';
    throw new Unanswered(reason, result.result);
  }
  if (!result.result.trim()) throw new Unanswered('回答是空的', 'empty result');
  // `deepseek-flash[1m]` names the model with Claude Code's suffix for a 1M context window
  return { text: result.result.trim(), model: model.replace(/\[[^\]]*\]$/, '') };
}

function withFooter(text, model) {
  const body = text.length > MAX_REPLY ? `${text.slice(0, MAX_REPLY)}\n\n……（回答太长，后面的部分在运行记录里）` : text;
  return `${body}\n\n<sub>${model} · [运行记录](${runUrl})</sub>`;
}

async function main() {
  const ask = readAsk();
  applyConfig(ask);
  try {
    if (!(await canWrite(ask.author))) {
      console.log(`@${ask.author} can't write to the repository: not answering`);
      return;
    }
    const installed = install();
    // awaited below; this only keeps a failure before then from being an unhandled rejection
    installed.catch(() => {});
    await react(ask).catch((err) => console.log(`::warning::${escape(`no 👀: ${err.message}`)}`));
    if (!process.env.ANTHROPIC_API_KEY && !process.env.ANTHROPIC_AUTH_TOKEN) {
      throw new Unanswered('没有设置 API Key', 'neither the ANTHROPIC_API_KEY nor the ANTHROPIC_AUTH_TOKEN secret is set');
    }
    const thread = await readThread(ask);
    if (thread.pr) fetchPrHead(thread.pr);
    console.log(`::group::prompt\n${prompt(ask, thread)}\n::endgroup::`);
    await installed;
    const { query } = await import('@anthropic-ai/claude-agent-sdk');
    const { text, model } = await answer(query, ask, thread);
    console.log(`::group::answer\n${text}\n::endgroup::`);
    await reply(ask, withFooter(text, model));
  } catch (err) {
    console.log(`::error::${escape(err.stack ?? String(err))}`);
    const reason = err instanceof Unanswered ? err.reason : '运行出错';
    await reply(ask, `没有回答（${reason}），详见[运行记录](${runUrl})。`);
    process.exitCode = 1;
  }
}

await main();
// a leftover handle or child process (npm after an early failure) mustn't hold the job until its timeout
process.exit();
