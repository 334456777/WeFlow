// A small client of the GitHub REST API for the repository the workflow runs in, authenticated by the job's token.

const API_VERSION = "2022-11-28";

/** A response of the REST API with an error status. */
export class HttpError extends Error {
  /** The HTTP status. */
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

/** Requests to the REST API under `/repos/<owner>/<repo>/`. */
export class GitHub {
  readonly #base: string;
  readonly #token: string;

  /**
   * @param repository `owner/repo`
   * @param token a token that may read the repository and write issues and pull requests
   * @param apiUrl the REST API's root, `GITHUB_API_URL` in a workflow
   */
  constructor(repository: string, token: string, apiUrl = "https://api.github.com") {
    this.#base = `${apiUrl}/repos/${repository}/`;
    this.#token = token;
  }

  async #fetch(url: string, init: RequestInit = {}, accept = "application/vnd.github+json"): Promise<Response> {
    const response = await fetch(url, {
      ...init,
      headers: {
        Accept: accept,
        Authorization: `Bearer ${this.#token}`,
        "X-GitHub-Api-Version": API_VERSION,
        "User-Agent": "weflow-claude",
        ...(init.body === undefined ? {} : { "Content-Type": "application/json" }),
      },
    });
    if (!response.ok) {
      const message = `${init.method ?? "GET"} ${url}: HTTP ${response.status} ${await response.text()}`;
      throw new HttpError(message, response.status);
    }
    return response;
  }

  /** Reads one JSON resource. */
  async get<T>(path: string): Promise<T> {
    return (await (await this.#fetch(this.#base + path)).json()) as T;
  }

  /** Reads every page of a list resource. */
  async list<T>(path: string): Promise<T[]> {
    const items: T[] = [];
    let url: string | undefined = `${this.#base}${path}${path.includes("?") ? "&" : "?"}per_page=100`;
    while (url !== undefined) {
      const response = await this.#fetch(url);
      items.push(...((await response.json()) as T[]));
      url = /<([^>]+)>;\s*rel="next"/.exec(response.headers.get("Link") ?? "")?.[1];
    }
    return items;
  }

  /** Reads a resource in another media type, such as `application/vnd.github.diff`. */
  async text(path: string, accept: string): Promise<string> {
    return (await this.#fetch(this.#base + path, {}, accept)).text();
  }

  /** Creates a resource. */
  async post(path: string, body: object): Promise<void> {
    await this.#fetch(this.#base + path, { method: "POST", body: JSON.stringify(body) });
  }

  /** The user's permission on the repository: `admin`, `write`, `read` or `none`. */
  async permission(user: string): Promise<string> {
    try {
      return (await this.get<{ permission: string }>(`collaborators/${encodeURIComponent(user)}/permission`))
        .permission;
    } catch (error) {
      // a user who is not a collaborator, or no longer exists
      if (error instanceof HttpError && error.status === 404) return "none";
      throw error;
    }
  }
}
