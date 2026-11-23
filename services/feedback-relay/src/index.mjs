import { Github, GithubRejected } from "./github.mjs";
import { feedback, reply } from "./validation.mjs";
import { HttpError, configuration, digestSecret, constantEqual, randomId, json, errorResponse,
  readJson, bearer, publicText, publicTitle, diagnosticText, MAX_REQUEST_BYTES, encoder } from "./security.mjs";

const PREFIX = { device: "设备适配", battery: "电量显示", dpi: "DPI 设置", other: "其他反馈" };
const PENDING = { status: "pending", retryAfter: 5 };
const HOUR = 60 * 60 * 1000;
const DAY = 24 * HOUR;

export default {
  async fetch(request, env) {
    try {
      configuration(env);
      const url = new URL(request.url);
      if (url.pathname === "/health" && request.method === "GET") return json({ status: "ok", schema: 1 });
      if (/^\/attachments\/[a-f0-9]{48}$/.test(url.pathname) && ["GET", "HEAD"].includes(request.method)) {
        const item = await env.ATTACHMENTS.get(url.pathname.slice(1));
        if (!item) throw new HttpError(404, "not_found", "Attachment not found.");
        const headers = new Headers({
          "Content-Type": item.httpMetadata?.contentType ?? "application/octet-stream",
          "Content-Length": String(item.size),
          "Content-Disposition": "inline",
          "Cache-Control": "public, max-age=86400, immutable",
          "X-Content-Type-Options": "nosniff",
          "Content-Security-Policy": "default-src 'none'; sandbox",
          "Referrer-Policy": "no-referrer",
        });
        return new Response(request.method === "HEAD" ? null : item.body, { headers });
      }
      const declared = request.headers.get("Content-Length");
      if (declared !== null && (!/^\d+$/.test(declared) || Number(declared) > MAX_REQUEST_BYTES)) {
        throw new HttpError(413, "request_too_large", "Request is too large.");
      }
      if (!(url.pathname === "/feedback" && request.method === "POST")
        && !(/^\/feedback\/[1-9][0-9]{0,9}$/.test(url.pathname) && request.method === "GET")
        && !(/^\/feedback\/[1-9][0-9]{0,9}\/replies$/.test(url.pathname) && request.method === "POST")) {
        throw new HttpError(404, "not_found", "Endpoint not found.");
      }
      const ip = request.headers.get("CF-Connecting-IP");
      if (!ip) throw new HttpError(503, "client_address", "Client address is unavailable.", true);
      const forwarded = new Request(request);
      // This header is overwritten at the public boundary. Caller-supplied
      // proxy headers cannot choose a different rate-limit identity.
      forwarded.headers.set("X-Relay-Ip", await digestSecret(env.FEEDBACK_SECRET, "ip", ip));
      return await env.FEEDBACK.get(env.FEEDBACK.idFromName("coordinator-v1")).fetch(forwarded);
    } catch (error) { return errorResponse(error); }
  },
};

export class FeedbackCoordinator {
  constructor(state, env, dependencies = {}) {
    this.storage = state.storage;
    this.env = env;
    this.now = dependencies.now ?? Date.now;
    this.github = new Github(env.GITHUB_TOKEN, env.GITHUB_AUTHOR_LOGIN, dependencies.fetch ?? fetch);
    // Keep one mutation in flight. Durable storage journals the request before
    // any GitHub POST so a new instance after a restart cannot repeat it.
    this.queue = Promise.resolve();
  }

  exclusive(action) {
    const result = this.queue.then(action, action);
    this.queue = result.catch(() => {});
    return result;
  }

  fetch(request) {
    return this.exclusive(async () => {
      try {
        configuration(this.env);
        const ip = request.headers.get("X-Relay-Ip");
        if (!/^[a-f0-9]{64}$/.test(ip ?? "")) throw new HttpError(403, "forbidden", "Invalid relay route.");
        await this.limits([
          [`requests:${ip}`, 60, 10 * 60 * 1000],
          ["requests:global", 5000, DAY],
        ]);
        const url = new URL(request.url);
        if (request.method === "POST" && url.pathname === "/feedback") {
          return await this.submit(await feedback(await readJson(request)), ip);
        }
        const match = /^\/feedback\/([1-9][0-9]{0,9})(\/replies)?$/.exec(url.pathname);
        if (!match) throw new HttpError(404, "not_found", "Endpoint not found.");
        const number = Number(match[1]);
        const key = bearer(request);
        const owner = await digestSecret(this.env.FEEDBACK_SECRET, "reply-key", key);
        const stored = await this.storage.get(`issue:${number}`);
        if (!stored || !constantEqual(stored.owner, owner)) throw new HttpError(403, "forbidden", "This reply key does not match the feedback.");
        if (request.method === "POST" && match[2]) {
          return await this.submitReply(number, await reply(await readJson(request, 20_000)), key, owner, ip);
        }
        if (request.method === "GET" && !match[2]) return await this.thread(number, stored);
        throw new HttpError(404, "not_found", "Endpoint not found.");
      } catch (error) { return errorResponse(error); }
    });
  }

  async limits(rules) {
    const now = this.now();
    const updates = [];
    for (const [scope, maximum, period] of rules) {
      const bucket = Math.floor(now / period);
      const key = `rate:${scope}:${bucket}`;
      const previous = await this.storage.get(key);
      const count = previous?.count ?? 0;
      if (count >= maximum) throw new HttpError(429, "rate_limited", "Too many requests. Try again later.", true,
        Math.max(1, Math.ceil(((bucket + 1) * period - now) / 1000)));
      updates.push([key, { count: count + 1, expiresAt: (bucket + 2) * period }]);
    }
    for (const [key, value] of updates) await this.storage.put(key, value);
    await this.schedule();
  }

  async schedule(when = this.now() + HOUR) {
    const current = await this.storage.getAlarm();
    if (current === null || current > when) await this.storage.setAlarm(when);
  }

  async submit(data, ip) {
    const recordKey = `request:${data.requestId}`;
    const owner = await digestSecret(this.env.FEEDBACK_SECRET, "reply-key", data.replyKey);
    let record = await this.storage.get(recordKey);
    if (record) {
      if (!constantEqual(record.owner, owner) || record.fingerprint !== data.fingerprint) {
        throw new HttpError(409, "request_conflict", "requestId was already used with different content or credentials.");
      }
      if (record.state === "complete") return json(record.receipt);
      if (record.state === "rejected") throw new HttpError(502, "github_rejected", "GitHub rejected this submission. Contact the app maintainer.");
      if (record.state !== "uploading" && record.state !== "prepared") return this.pending(recordKey, record);
    } else {
      await this.limits([[`new:${ip}`, 3, HOUR], [`new-day:${ip}`, 8, DAY], ["new:global", 200, DAY]]);
      const marker = `<!-- r-battery-feedback:${await digestSecret(this.env.FEEDBACK_SECRET, "issue", data.requestId)} -->`;
      const attachments = data.images.map(image => ({ id: randomId(), mimeType: image.mimeType, digest: image.digest }));
      record = { kind: "issue", state: "uploading", fingerprint: data.fingerprint, owner, marker,
        attachments, createdAt: this.now(), nextCheckAt: 0, checks: 0 };
      await this.storage.put(recordKey, record);
    }
    if (record.state === "uploading") {
      for (let index = 0; index < data.images.length; index++) {
        const image = data.images[index];
        const saved = record.attachments[index];
        await this.env.ATTACHMENTS.put(`attachments/${saved.id}`, image.bytes, {
          httpMetadata: { contentType: image.mimeType },
          customMetadata: { createdAt: String(record.createdAt) },
        });
      }
      const { base } = configuration(this.env);
      const safe = value => publicText(value, data.replyKey);
      const title = `[${PREFIX[data.category]}] ${publicTitle(data.device.name, data.replyKey)}`;
      const sections = [
        "通过 R Battery 提交的用户反馈。",
        `应用版本：${safe(data.appVersion ?? "未提供")}`,
        `设备：${safe(data.device.name)}\n类型：${data.device.family}\n连接方式：${safe(data.device.connection)}`,
      ];
      if (data.device.brand) sections.push(`品牌：${safe(data.device.brand)}`);
      if (data.device.model) sections.push(`型号：${safe(data.device.model)}`);
      sections.push(`### 问题描述\n\n${safe(data.description)}`);
      if (data.diagnostics) sections.push(`### 诊断信息\n\n${diagnosticText(data.diagnostics, data.replyKey)}`);
      if (record.attachments.length) sections.push("### 截图\n\n" + record.attachments.map((image, index) =>
        `![截图 ${index + 1}](${base}/attachments/${image.id})`).join("\n\n"));
      sections.push(record.marker);
      record = { ...record, state: "prepared", title, body: sections.join("\n\n") };
      if (encoder.encode(record.body).byteLength > 60_000 || record.title.length > 250) {
        throw new HttpError(400, "rendered_too_large", "Rendered feedback is too large. Shorten the diagnostics or device name.");
      }
      await this.storage.put(recordKey, record);
    }
    return this.send(recordKey, record);
  }

  async submitReply(number, data, replyKey, owner, ip) {
    const recordKey = `reply:${number}:${data.requestId}`;
    let record = await this.storage.get(recordKey);
    if (record) {
      if (record.fingerprint !== data.fingerprint || !constantEqual(record.owner, owner)) {
        throw new HttpError(409, "request_conflict", "requestId was already used with different content.");
      }
      if (record.state === "complete") return json(record.receipt);
      if (record.state === "rejected") throw new HttpError(502, "github_rejected", "GitHub rejected this reply. Contact the app maintainer.");
      if (record.state !== "prepared") return this.pending(recordKey, record);
    } else {
      await this.limits([[`reply:${number}`, 20, DAY], [`reply-ip:${ip}`, 60, DAY], ["reply:global", 1000, DAY]]);
      const marker = `<!-- r-battery-reply:${await digestSecret(this.env.FEEDBACK_SECRET, "comment", `${number}:${data.requestId}`)} -->`;
      record = { kind: "reply", state: "prepared", number, owner, fingerprint: data.fingerprint, marker,
        body: `**反馈者补充**\n\n${publicText(data.body, replyKey)}\n\n${marker}`,
        createdAt: this.now(), nextCheckAt: 0, checks: 0 };
      await this.storage.put(recordKey, record);
    }
    return this.send(recordKey, record);
  }

  async send(recordKey, record) {
    // Persist sending before the external side effect. After this point neither
    // a client retry nor an alarm may issue a second POST for the same request.
    const { title, body, ...journal } = record;
    record = { ...journal, state: "sending", nextCheckAt: this.now() + 5_000 };
    await this.storage.put(recordKey, record);
    await this.schedule(record.nextCheckAt);
    try {
      const receipt = record.kind === "issue"
        ? await this.github.createIssue(title, body)
        : await this.github.createReply(record.number, body);
      await this.complete(recordKey, record, receipt);
      return json(receipt, 201);
    } catch (error) {
      if (error instanceof GithubRejected) {
        await this.storage.put(recordKey, { ...record, state: "rejected", rejectedAt: this.now() });
        throw new HttpError(502, "github_rejected", "GitHub rejected this submission. Contact the app maintainer.");
      }
      // A transport failure can occur after GitHub accepted the write.
      await this.storage.put(recordKey, { ...record, state: "pending" });
      return json(PENDING, 202, { "Retry-After": "5" });
    }
  }

  async complete(recordKey, record, receipt) {
    if (record.kind === "issue") {
      const existing = await this.storage.get(`issue:${receipt.number}`);
      if (existing && !constantEqual(existing.owner, record.owner)) throw new Error("Issue ownership conflict.");
      await this.storage.put(`issue:${receipt.number}`, { owner: record.owner, createdAt: record.createdAt });
    } else {
      const issue = await this.storage.get(`issue:${record.number}`);
      if (issue) await this.storage.put(`issue:${record.number}`, { owner: issue.owner, createdAt: issue.createdAt });
    }
    const { title, body, ...minimal } = record;
    await this.storage.put(recordKey, { ...minimal, state: "complete", receipt, completedAt: this.now() });
  }

  async pending(recordKey, record) {
    if (this.now() >= record.nextCheckAt) {
      const recovered = await this.reconcile(recordKey, record);
      if (recovered) return json(recovered);
      record = await this.storage.get(recordKey);
    }
    const retryAfter = Math.max(5, Math.ceil((record.nextCheckAt - this.now()) / 1000));
    return json({ status: "pending", retryAfter }, 202, { "Retry-After": String(retryAfter) });
  }

  async reconcile(recordKey, record) {
    try {
      const receipt = record.kind === "issue" ? await this.github.findIssue(record.marker, record.createdAt)
        : await this.github.findReply(record.number, record.marker);
      if (receipt) { await this.complete(recordKey, record, receipt); return receipt; }
    } catch { /* Keep the journal; no blind retry after an uncertain write. */ }
    const checks = record.checks + 1;
    const nextCheckAt = this.now() + Math.min(HOUR, 5_000 * 2 ** Math.min(checks, 10));
    await this.storage.put(recordKey, { ...record, state: "pending", checks, nextCheckAt });
    await this.schedule(nextCheckAt);
    return null;
  }

  async thread(number, stored) {
    if (stored.cache && stored.cacheUntil > this.now()) return json(stored.cache);
    let result;
    try { result = await this.github.thread(number); }
    catch (error) {
      if (error instanceof GithubRejected && [404, 410].includes(error.status)) {
        throw new HttpError(404, "not_found", "The GitHub issue is no longer available.");
      }
      throw new HttpError(503, "github_unavailable", "Could not load feedback updates. Try again later.", true);
    }
    if (encoder.encode(JSON.stringify(result)).byteLength < 96 * 1024) {
      await this.storage.put(`issue:${number}`, { ...stored, cache: result, cacheUntil: this.now() + 30_000 });
    }
    return json(result);
  }

  alarm() {
    return this.exclusive(async () => {
      const now = this.now();
      const rateCursor = await this.storage.get("scan:rate");
      const expiredRates = await this.storage.list({ prefix: "rate:", limit: 1000,
        ...(rateCursor ? { startAfter: rateCursor } : {}) });
      if (expiredRates.size === 1000) await this.storage.put("scan:rate", [...expiredRates.keys()].at(-1));
      else await this.storage.delete("scan:rate");
      for (const [key, value] of expiredRates) {
        if (value.expiresAt < now) await this.storage.delete(key);
      }
      let checked = 0;
      let next = now + (expiredRates.size === 1000 ? 60_000 : HOUR);
      for (const prefix of ["request:", "reply:"]) {
        const cursor = await this.storage.get(`scan:${prefix}`);
        const rows = await this.storage.list({ prefix, limit: 250, ...(cursor ? { startAfter: cursor } : {}) });
        if (rows.size === 250) {
          await this.storage.put(`scan:${prefix}`, [...rows.keys()].at(-1));
          next = Math.min(next, now + 60_000);
        } else await this.storage.delete(`scan:${prefix}`);
        for (const [key, record] of rows) {
          if (["sending", "pending"].includes(record.state)) {
            if (record.nextCheckAt <= now && checked++ < 3) await this.reconcile(key, record);
            else next = Math.min(next, Math.max(now + 5_000, record.nextCheckAt));
          }
          if (["uploading", "prepared", "rejected"].includes(record.state) && record.createdAt + DAY < now
            && (record.state !== "rejected" || record.attachments?.length)) {
            for (const image of record.attachments ?? []) await this.env.ATTACHMENTS.delete(`attachments/${image.id}`);
            const { title, body, ...minimal } = record;
            await this.storage.put(key, { ...minimal, state: "rejected", attachments: [] });
          }
        }
      }
      // Alarm callbacks replace their own due time; retaining the fired value
      // would cause scheduled cleanup/reconciliation to stop after one run.
      const scheduled = await this.storage.getAlarm();
      await this.storage.setAlarm(scheduled > now ? Math.min(scheduled, next) : next);
    });
  }
}
