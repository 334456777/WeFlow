// The mention of @claude a GitHub event carries: who asked what, in which issue or pull request, and where the
// acknowledgement and the answer go. The `if` of claude.yml lets through only events whose text mentions @claude.

/** A mention of @claude and the places the answer goes. */
export interface Mention {
  /** The issue or pull request the mention is in. */
  number: number;
  /** Whether `number` is a pull request. */
  isPullRequest: boolean;
  /** The login of the user who wrote the mention. */
  asker: string;
  /** The text that mentions @claude. */
  text: string;
  /** For a review comment, the file, line and diff hunk it is on. */
  location?: string;
  /** The REST path (under the repository) of the reactions that acknowledge the mention; reviews have none. */
  reactionsPath?: string;
  /** The REST path (under the repository) the answer is posted to. */
  replyPath: string;
}

interface User {
  login: string;
}

interface Comment {
  id: number;
  body: string | null;
  path?: string;
  line?: number | null;
  original_line?: number | null;
  diff_hunk?: string;
}

/** The fields of the webhook payloads of claude.yml's events that a mention is read from. */
export interface GitHubEvent {
  sender: User;
  issue?: { number: number; title: string; body: string | null; pull_request?: unknown };
  pull_request?: { number: number };
  comment?: Comment;
  review?: { body: string | null };
}

function required<T>(value: T | undefined, what: string): T {
  if (value === undefined) throw new Error(`the event has no ${what}`);
  return value;
}

/** Reads the mention from the payload of a claude.yml event. */
export function readMention(eventName: string, event: GitHubEvent): Mention {
  const asker = event.sender.login;
  switch (eventName) {
    case "issue_comment": {
      const issue = required(event.issue, "issue");
      const comment = required(event.comment, "comment");
      return {
        number: issue.number,
        isPullRequest: issue.pull_request !== undefined,
        asker,
        text: comment.body ?? "",
        reactionsPath: `issues/comments/${comment.id}/reactions`,
        replyPath: `issues/${issue.number}/comments`,
      };
    }
    case "pull_request_review_comment": {
      const number = required(event.pull_request, "pull request").number;
      const comment = required(event.comment, "comment");
      return {
        number,
        isPullRequest: true,
        asker,
        text: comment.body ?? "",
        location: `${comment.path}, line ${comment.line ?? comment.original_line}:\n\`\`\`diff\n${comment.diff_hunk}\n\`\`\``,
        reactionsPath: `pulls/comments/${comment.id}/reactions`,
        replyPath: `pulls/${number}/comments/${comment.id}/replies`,
      };
    }
    case "pull_request_review": {
      const number = required(event.pull_request, "pull request").number;
      return {
        number,
        isPullRequest: true,
        asker,
        text: required(event.review, "review").body ?? "",
        replyPath: `issues/${number}/comments`,
      };
    }
    case "issues": {
      const issue = required(event.issue, "issue");
      return {
        number: issue.number,
        isPullRequest: false,
        asker,
        text: `${issue.title}\n\n${issue.body ?? ""}`,
        reactionsPath: `issues/${issue.number}/reactions`,
        replyPath: `issues/${issue.number}/comments`,
      };
    }
    default:
      throw new Error(`claude.yml does not handle ${eventName} events`);
  }
}
