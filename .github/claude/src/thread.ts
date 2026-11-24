// The issue or pull request a mention is in, as text for the prompt: its description and discussion, and for a pull
// request its reviews, review comments, changed files and diff (the checkout Claude reads is the default branch).

import type { GitHub } from "./github.ts";
import type { Mention } from "./event.ts";

/** Diffs longer than this many characters are cut, keeping the prompt within the model's context. */
const MAX_DIFF = 200_000;

interface User {
  login: string;
}

interface Issue {
  title: string;
  body: string | null;
  user: User;
  state: string;
  labels: { name: string }[];
}

interface IssueComment {
  user: User;
  created_at: string;
  body: string | null;
}

interface PullRequest {
  head: { ref: string };
  base: { ref: string };
}

interface Review {
  user: User;
  state: string;
  submitted_at: string;
  body: string | null;
}

interface ReviewComment {
  user: User;
  path: string;
  line: number | null;
  original_line: number | null;
  created_at: string;
  body: string;
}

interface ChangedFile {
  filename: string;
  additions: number;
  deletions: number;
}

function entry(user: User, when: string, body: string | null, label = ""): string {
  return `### @${user.login}${label}, ${when}\n${body ?? ""}`;
}

/** Describes the issue or pull request of the mention. */
export async function describeThread(github: GitHub, mention: Mention): Promise<string> {
  const n = mention.number;
  const [issue, comments] = await Promise.all([
    github.get<Issue>(`issues/${n}`),
    github.list<IssueComment>(`issues/${n}/comments`),
  ]);
  const kind = mention.isPullRequest ? "Pull request" : "Issue";
  const parts = [
    `${kind} #${n}: ${issue.title}`,
    `Author: @${issue.user.login}; state: ${issue.state}; labels: ${issue.labels.map((l) => l.name).join(", ")}`,
    issue.body ?? "",
    "## Comments",
    ...comments.map((c) => entry(c.user, c.created_at, c.body)),
  ];
  if (mention.isPullRequest) {
    const [pull, reviews, reviewComments, files, diff] = await Promise.all([
      github.get<PullRequest>(`pulls/${n}`),
      github.list<Review>(`pulls/${n}/reviews`),
      github.list<ReviewComment>(`pulls/${n}/comments`),
      github.list<ChangedFile>(`pulls/${n}/files`),
      github.text(`pulls/${n}`, "application/vnd.github.diff"),
    ]);
    parts.push(
      `## Branches\n${pull.head.ref} into ${pull.base.ref}`,
      "## Reviews",
      ...reviews.filter((r) => r.body).map((r) => entry(r.user, r.submitted_at, r.body, ` (${r.state})`)),
      "## Review comments",
      ...reviewComments.map((c) => entry(c.user, c.created_at, c.body, ` on ${c.path}:${c.line ?? c.original_line}`)),
      "## Changed files",
      files.map((f) => `- ${f.filename} (+${f.additions} -${f.deletions})`).join("\n"),
      "## Diff",
      "```diff\n" +
        (diff.length > MAX_DIFF ? `${diff.slice(0, MAX_DIFF)}\n[the diff is cut here]` : diff) +
        "\n```",
    );
  }
  return parts.join("\n\n");
}
