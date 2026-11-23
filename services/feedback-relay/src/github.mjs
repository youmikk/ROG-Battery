import { API_ORIGIN, REPOSITORY, HttpError, checkedIssueNumber, checkGithubUrl, encoder } from "./security.mjs";

export class GithubRejected extends Error {
  constructor(status) { super("GitHub rejected the request."); this.status = status; }
}

export class Github {
  constructor(token, author, fetchImpl = fetch) {
    this.token = token;
    this.author = author.toLowerCase();
    this.fetch = fetchImpl;
  }

  authored(value) { return value?.user?.login?.toLowerCase() === this.author; }

  async request(path, body) {
    // path is built exclusively by this class; credentials never follow redirects.
    const response = await this.fetch(`${API_ORIGIN}/repos/${REPOSITORY}${path}`, {
      method: body === undefined ? "GET" : "POST",
      redirect: "error",
      headers: {
        "Authorization": `Bearer ${this.token}`,
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "R-Battery-Feedback",
        ...(body === undefined ? {} : { "Content-Type": "application/json" }),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(8_000),
    });
    if (!response.ok) {
      // Only an explicit rejection is safe to distinguish from a lost POST
      // response. 5xx/timeouts remain uncertain and are reconciled, never resent.
      if ([400, 401, 403, 404, 410, 422, 429].includes(response.status)) throw new GithubRejected(response.status);
      throw new HttpError(502, "github_unavailable", "GitHub is temporarily unavailable.", true);
    }
    const value = await response.json();
    if (body !== undefined && response.status !== 201) throw new Error("Unexpected GitHub write response.");
    return value;
  }

  issueReceipt(value) {
    const number = checkedIssueNumber(value?.number);
    if (value.pull_request) throw new Error("Expected an issue.");
    return { number, url: checkGithubUrl(value.html_url, number), replyAccess: true };
  }

  commentReceipt(value, number) {
    const id = checkedIssueNumber(value?.id);
    if (typeof value.created_at !== "string" || !Number.isFinite(Date.parse(value.created_at))) throw new Error("Invalid comment timestamp.");
    return { id, url: checkGithubUrl(value.html_url, number, id), createdAt: value.created_at };
  }

  async createIssue(title, body) {
    const value = await this.request("/issues", { title, body });
    if (!this.authored(value)) throw new Error("Unexpected issue author.");
    return this.issueReceipt(value);
  }
  async createReply(number, body) {
    const value = await this.request(`/issues/${number}/comments`, { body });
    if (!this.authored(value)) throw new Error("Unexpected comment author.");
    return this.commentReceipt(value, number);
  }

  async findIssue(marker, since) {
    // Do not depend on GitHub Search indexing to recover an interrupted write.
    const threshold = since - 60_000;
    for (let page = 1; page <= 5; page++) {
      const rows = await this.request(`/issues?state=all&sort=created&direction=desc&per_page=100&page=${page}`);
      if (!Array.isArray(rows)) throw new Error("Invalid issue list.");
      for (const row of rows) {
        if (!row.pull_request && this.authored(row) && typeof row.body === "string"
          && row.body.includes(marker)) return this.issueReceipt(row);
      }
      if (rows.length < 100 || rows.some(row => Date.parse(row.created_at) < threshold)) return null;
    }
    return null;
  }

  async issue(number) {
    const value = await this.request(`/issues/${number}`);
    if (value.number !== number || value.pull_request) throw new Error("Unexpected issue.");
    return value;
  }

  async recentComments(number, count, maxPages = 3) {
    if (!Number.isSafeInteger(count) || count < 0) throw new Error("Invalid comment count.");
    const last = Math.max(1, Math.ceil(count / 100));
    const first = Math.max(1, last - maxPages + 1);
    const comments = [];
    for (let page = first; page <= last; page++) {
      const rows = await this.request(`/issues/${number}/comments?per_page=100&page=${page}`);
      if (!Array.isArray(rows)) throw new Error("Invalid comment list.");
      comments.push(...rows);
    }
    return { comments, truncated: first > 1 };
  }

  async findReply(number, marker) {
    const issue = await this.issue(number);
    const { comments } = await this.recentComments(number, issue.comments ?? 0, 5);
    const found = comments.find(row => this.authored(row) && typeof row.body === "string" && row.body.includes(marker));
    return found ? this.commentReceipt(found, number) : null;
  }

  async thread(number) {
    const value = await this.issue(number);
    const { comments, truncated } = await this.recentComments(number, value.comments ?? 0);
    const result = {
      issue: {
        number,
        url: checkGithubUrl(value.html_url, number),
        title: String(value.title ?? ""),
        state: value.state === "closed" ? "closed" : "open",
        body: String(value.body ?? "").replace(/\n?<!-- r-battery-feedback:[a-f0-9]{64} -->/g, ""),
        createdAt: value.created_at,
        updatedAt: value.updated_at,
      },
      comments: comments.filter(row => ["OWNER", "MEMBER", "COLLABORATOR"].includes(row.author_association)
        && typeof row.body === "string" && !/<!-- r-battery-reply:[a-f0-9]{64} -->/.test(row.body))
        .map(row => ({ ...this.commentReceipt(row, number), body: row.body,
          author: String(row.user?.login ?? ""), isMaintainer: true })),
      commentsTruncated: truncated,
    };
    while (result.comments.length && encoder.encode(JSON.stringify(result)).byteLength > 500 * 1024) {
      result.comments.shift();
      result.commentsTruncated = true;
    }
    if (encoder.encode(JSON.stringify(result)).byteLength > 500 * 1024) throw new Error("Oversized issue response.");
    return result;
  }
}
