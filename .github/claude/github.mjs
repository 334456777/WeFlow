// The GitHub side of the @claude answers: the request the triggering event carries, who may ask, the thread read
// for the prompt, the 👀 reaction and the reply. REST calls use the job's GITHUB_TOKEN (GH_TOKEN). With
// CLAUDE_DRY_RUN=1 the reaction and the reply are printed instead of posted, for trying the script outside Actions.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const API = process.env.GITHUB_API_URL ?? 'https://api.github.com';
const REPO = process.env.GITHUB_REPOSITORY;
const DRY_RUN = process.env.CLAUDE_DRY_RUN === '1';

/** The run's page on GitHub, linked from every reply. */
export const runUrl = `${process.env.GITHUB_SERVER_URL ?? 'https://github.com'}/${REPO}/actions/runs/${process.env.GITHUB_RUN_ID}`;

async function call(method, path, body) {
  const response = await fetch(path.startsWith('https://') ? path : `${API}${path}`, {
    method,
    headers: {
      Accept: 'application/vnd.github+json',
      Authorization: `Bearer ${process.env.GH_TOKEN}`,
      'User-Agent': 'weflow-claude',
      'X-GitHub-Api-Version': '2022-11-28',
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) throw new Error(`${method} ${path}: HTTP ${response.status} ${await response.text()}`);
  return { data: response.status === 204 ? null : await response.json(), link: response.headers.get('link') };
}

const get = async (path) => (await call('GET', path)).data;

async function getAll(path) {
  const items = [];
  let next = `${path}${path.includes('?') ? '&' : '?'}per_page=100`;
  while (next) {
    const { data, link } = await call('GET', next);
    items.push(...data);
    next = link?.match(/<([^>]+)>;\s*rel="next"/)?.[1];
  }
  return items;
}

/**
 * @typedef {object} Ask
 * @property {number} number the issue or pull request asked in
 * @property {string} author login of who asked
 * @property {string | null} question the text that mentions @claude; null when it is the issue itself
 * @property {string | null} reactOn the reactions endpoint of what mentions @claude; reviews and manual runs have none
 * @property {number | null} reviewRoot the top-level review comment of the thread asked in, which the reply goes under
 * @property {number | null} reviewId the review whose body asked
 */

/** Reads the request out of the event that started the run. @returns {Ask} */
export function readAsk() {
  const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
  const ask = { question: null, reactOn: null, reviewRoot: null, reviewId: null };
  switch (process.env.GITHUB_EVENT_NAME) {
    case 'issue_comment':
      return {
        ...ask,
        number: event.issue.number,
        author: event.comment.user.login,
        question: event.comment.body,
        reactOn: `/repos/${REPO}/issues/comments/${event.comment.id}/reactions`,
      };
    case 'pull_request_review_comment':
      return {
        ...ask,
        number: event.pull_request.number,
        author: event.comment.user.login,
        question: event.comment.body,
        reactOn: `/repos/${REPO}/pulls/comments/${event.comment.id}/reactions`,
        reviewRoot: event.comment.in_reply_to_id ?? event.comment.id,
      };
    case 'pull_request_review':
      return {
        ...ask,
        number: event.pull_request.number,
        author: event.review.user.login,
        question: event.review.body,
        reviewId: event.review.id,
      };
    case 'issues':
      return {
        ...ask,
        number: event.issue.number,
        author: event.issue.user.login,
        reactOn: `/repos/${REPO}/issues/${event.issue.number}/reactions`,
      };
    case 'workflow_dispatch':
      return {
        ...ask,
        number: Number(event.inputs.number),
        author: process.env.GITHUB_ACTOR,
        question: event.inputs.prompt,
      };
    default:
      throw new Error(`unexpected event ${process.env.GITHUB_EVENT_NAME}`);
  }
}

/** Whether `login` may push to the repository: only they are answered. */
export async function canWrite(login) {
  const { permission } = await get(`/repos/${REPO}/collaborators/${encodeURIComponent(login)}/permission`);
  return permission === 'admin' || permission === 'write';
}

/** Marks what asked with 👀, so the asker sees it was picked up. */
export async function react(ask) {
  if (!ask.reactOn) return;
  if (DRY_RUN) return console.log(`[dry run] 👀 on ${ask.reactOn}`);
  await call('POST', ask.reactOn, { content: 'eyes' });
}

/**
 * The issue or pull request, its comments in order and, when a review asked, the review thread or the review's
 * inline comments.
 */
export async function readThread(ask) {
  const issue = await get(`/repos/${REPO}/issues/${ask.number}`);
  const comments = await getAll(`/repos/${REPO}/issues/${ask.number}/comments`);
  const pr = issue.pull_request ? await get(`/repos/${REPO}/pulls/${ask.number}`) : null;
  let review = [];
  if (ask.reviewRoot) {
    review = (await getAll(`/repos/${REPO}/pulls/${ask.number}/comments`)).filter(
      (c) => c.id === ask.reviewRoot || c.in_reply_to_id === ask.reviewRoot,
    );
  } else if (ask.reviewId) {
    review = await getAll(`/repos/${REPO}/pulls/${ask.number}/reviews/${ask.reviewId}/comments`);
  }
  return { issue, comments, pr, review };
}

/** The commit checked out in the working directory. */
export const checkoutSha = () => execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();

/**
 * Fetches the head of the pull request into the local branch `pr`, as objects only: its files are never checked
 * out, so a CLAUDE.md, settings or skills it adds or changes don't reach Claude Code.
 */
export function fetchPrHead(pr) {
  const depth = String(Math.min(Math.max(pr.commits, 1), 250));
  execFileSync('git', ['fetch', '--no-tags', `--depth=${depth}`, 'origin', `+refs/pull/${pr.number}/head:refs/heads/pr`], {
    stdio: 'inherit',
  });
}

/** Posts `body` where the question was asked: under the review thread, or as a comment on the issue or PR. */
export async function reply(ask, body) {
  const [path, what] = ask.reviewRoot
    ? [`/repos/${REPO}/pulls/${ask.number}/comments/${ask.reviewRoot}/replies`, `review thread ${ask.reviewRoot}`]
    : [`/repos/${REPO}/issues/${ask.number}/comments`, `#${ask.number}`];
  if (DRY_RUN) return console.log(`[dry run] reply on ${what}:\n${body}`);
  const { data } = await call('POST', path, { body });
  console.log(`replied: ${data.html_url}`);
}
