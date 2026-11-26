export const REPOSITORY = "youmikk/X-Battery";
export const API_ORIGIN = "https://api.github.com";
export const MAX_REQUEST_BYTES = 21 * 1024 * 1024;
export const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
export const MAX_CAPTURE_BYTES = 544 * 1024;
export const MAX_DIAGNOSTICS_BYTES = 20 * 1024;
export const encoder = new TextEncoder();

export class HttpError extends Error {
  constructor(status, code, message, retryable = false, retryAfter = 0) {
    super(message);
    this.status = status;
    this.code = code;
    this.retryable = retryable;
    this.retryAfter = retryAfter;
  }
}

export function json(value, status = 200, extraHeaders = {}) {
  return new Response(JSON.stringify(value), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
      "Referrer-Policy": "no-referrer",
      ...extraHeaders,
    },
  });
}

export function errorResponse(error) {
  const safe = error instanceof HttpError ? error
    : new HttpError(503, "service_unavailable", "Feedback service is temporarily unavailable.", true);
  return json({ error: { code: safe.code, message: safe.message, retryable: safe.retryable } },
    safe.status, safe.retryAfter ? { "Retry-After": String(safe.retryAfter) } : {});
}

export function configuration(env) {
  let base;
  try { base = new URL(env.PUBLIC_BASE_URL); } catch { /* handled below */ }
  if (!base || base.protocol !== "https:" || base.username || base.password
    || base.pathname !== "/" || base.search || base.hash || base.port
    || !env.GITHUB_TOKEN || !/^[a-fA-F0-9]{64}$/.test(env.FEEDBACK_SECRET ?? "")
    || !/^[A-Za-z0-9_-]{1,39}(?:\[bot\])?$/.test(env.GITHUB_AUTHOR_LOGIN ?? "")
    || !env.ATTACHMENTS) {
    throw new HttpError(503, "not_configured", "Feedback service has not been configured.");
  }
  return { base: base.origin };
}

export async function sha256(value) {
  const bytes = typeof value === "string" ? encoder.encode(value) : value;
  return hex(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)));
}

export async function digestSecret(secret, label, value) {
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret),
    { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return hex(new Uint8Array(await crypto.subtle.sign("HMAC", key, encoder.encode(`${label}:${value}`))));
}

export function hex(bytes) {
  return Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
}

export function randomId() {
  return hex(crypto.getRandomValues(new Uint8Array(24)));
}

export function constantEqual(a, b) {
  if (typeof a !== "string" || typeof b !== "string" || a.length !== b.length) return false;
  let diff = 0;
  for (let index = 0; index < a.length; index++) diff |= a.charCodeAt(index) ^ b.charCodeAt(index);
  return diff === 0;
}

export function bearer(request) {
  const match = /^Bearer ([a-fA-F0-9]{64})$/.exec(request.headers.get("Authorization") ?? "");
  if (!match) throw new HttpError(401, "unauthorized", "A valid reply key is required.");
  return match[1].toLowerCase();
}

export function issueUrl(number) {
  return `https://github.com/${REPOSITORY}/issues/${number}`;
}

export function checkedIssueNumber(value) {
  if (!Number.isSafeInteger(value) || value < 1) throw new HttpError(502, "invalid_upstream", "Invalid issue returned by GitHub.", true);
  return value;
}

export function checkGithubUrl(value, number, commentId) {
  let url;
  try { url = new URL(value); } catch { /* handled below */ }
  const hash = commentId === undefined ? "" : `#issuecomment-${commentId}`;
  if (!url || url.protocol !== "https:" || url.hostname !== "github.com" || url.port
    || url.username || url.password || url.search || url.hash !== hash
    || url.pathname.toLowerCase() !== `/${REPOSITORY}/issues/${number}`.toLowerCase()) {
    throw new HttpError(502, "invalid_upstream", "Unexpected issue URL returned by GitHub.", true);
  }
  return `${issueUrl(number)}${hash}`;
}

export async function readJson(request, maximum = MAX_REQUEST_BYTES, timeoutMs = 30_000, reserve = () => {}) {
  if (!/^application\/json(?:\s*;|$)/i.test(request.headers.get("Content-Type") ?? "")) {
    throw new HttpError(415, "content_type", "Content-Type must be application/json.");
  }
  if (request.headers.get("Content-Encoding") && request.headers.get("Content-Encoding") !== "identity") {
    throw new HttpError(415, "content_encoding", "Compressed requests are not supported.");
  }
  const declared = request.headers.get("Content-Length");
  if (declared !== null && (!/^\d+$/.test(declared) || Number(declared) > maximum)) {
    throw new HttpError(413, "request_too_large", "Request is too large.");
  }
  if (!request.body) throw new HttpError(400, "invalid_json", "JSON body is required.");
  const reader = request.body.getReader();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const parts = [];
  let size = 0;
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new HttpError(408, "request_timeout", "Request body upload timed out.", true)), timeoutMs);
  });
  try {
    while (true) {
      const { value, done } = await Promise.race([reader.read(), timeout]);
      if (done) break;
      size += value.byteLength;
      if (size > maximum) {
        throw new HttpError(413, "request_too_large", "Request is too large.");
      }
      reserve(value.byteLength);
      parts.push(decoder.decode(value, { stream: true }));
    }
    parts.push(decoder.decode());
    return JSON.parse(parts.join(""));
  } catch (error) {
    void reader.cancel().catch(() => {});
    if (error instanceof HttpError) throw error;
    throw new HttpError(400, "invalid_json", "Body must contain valid UTF-8 JSON.");
  } finally {
    clearTimeout(timer);
    reader.releaseLock();
  }
}

// User text is plain text, not GitHub HTML, mentions, task lists or image links.
export function publicTitle(value, replyKey) {
  return value.replace(new RegExp(replyKey, "ig"), "[redacted]").replace(/@/g, "@\u200b");
}

export function publicText(value, replyKey) {
  return value.replace(new RegExp(replyKey, "ig"), "[redacted]")
    .replace(/\r\n?/g, "\n").replace(/@/g, "@\u200b")
    .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    .replace(/([\\`*_{}\[\]()#+.!|~-])/g, "\\$1");
}

export function diagnosticText(value, replyKey) {
  const text = value.replace(new RegExp(replyKey, "ig"), "[redacted]")
    .replace(/\r\n?/g, "\n").replace(/@/g, "@\u200b");
  const backticks = Math.max(0, ...(text.match(/`+/g) ?? []).map(run => run.length));
  const tildes = Math.max(0, ...(text.match(/~+/g) ?? []).map(run => run.length));
  const fence = backticks <= tildes ? "`".repeat(backticks + 3) : "~".repeat(tildes + 3);
  return `${fence}text\n${text}\n${fence}`;
}
