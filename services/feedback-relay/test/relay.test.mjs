import test from "node:test";
import assert from "node:assert/strict";
import worker, { FeedbackCoordinator } from "../src/index.mjs";
import { Github } from "../src/github.mjs";
import { feedback, reply } from "../src/validation.mjs";
import { readJson, MAX_REQUEST_BYTES, MAX_IMAGE_BYTES, checkGithubUrl } from "../src/security.mjs";

const KEY = "23".repeat(32);
const IP = "ab".repeat(32);
const UUID = "45a526bf-b747-4d42-81e8-d1036d01f983";
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9ZfWQAAAAASUVORK5CYII=";
const BASE_TIME = Date.parse("2026-11-23T00:00:00Z");

class MemoryStorage {
  constructor() { this.data = new Map(); this.scheduled = null; }
  async get(key) { return structuredClone(this.data.get(key)); }
  async put(key, value) { this.data.set(key, structuredClone(value)); }
  async delete(key) { this.data.delete(key); }
  async list({ prefix, startAfter = "", limit = Infinity }) {
    return new Map([...this.data].filter(([key]) => key.startsWith(prefix) && key > startAfter)
      .sort(([a], [b]) => a.localeCompare(b)).slice(0, limit).map(([key, value]) => [key, structuredClone(value)]));
  }
  async getAlarm() { return this.scheduled; }
  async setAlarm(value) { this.scheduled = value; }
}

class MemoryR2 {
  constructor() { this.data = new Map(); this.puts = 0; }
  async put(key, bytes, metadata) { this.puts++; this.data.set(key, { bytes: bytes.slice(), ...metadata }); }
  async get(key) {
    const row = this.data.get(key);
    return row ? { body: row.bytes, size: row.bytes.length, httpMetadata: row.httpMetadata } : null;
  }
  async delete(key) { this.data.delete(key); }
}

class FakeGithub {
  constructor() {
    this.issues = [];
    this.comments = [];
    this.calls = [];
    this.loseIssueResponse = false;
    this.loseCommentResponse = false;
    this.reject = 0;
    this.noIssueCreated = false;
    this.nextIssue = 123;
  }

  fetch = async (input, init) => {
    const url = new URL(input);
    assert.equal(url.origin, "https://api.github.com");
    assert.ok(url.pathname.startsWith("/repos/youmikk/X-Battery/"));
    assert.equal(init.redirect, "manual");
    assert.equal(init.headers.Authorization, "Bearer fake-test-token");
    this.calls.push({ url: input, method: init.method, body: init.body });
    if (init.method === "POST" && this.reject) return new Response("{}", { status: this.reject });
    if (url.pathname.endsWith("/issues") && init.method === "POST") {
      if (this.noIssueCreated) throw new Error("Network failed before server response");
      const body = JSON.parse(init.body);
      const number = this.nextIssue++;
      const result = { number, html_url: `https://github.com/youmikk/X-Battery/issues/${number}`,
        title: body.title, body: body.body, state: "open", user: { login: "relay-bot" },
        created_at: "2026-11-23T00:00:00Z", updated_at: "2026-11-23T00:00:00Z", comments: 0 };
      this.issues.push(result);
      if (this.loseIssueResponse) { this.loseIssueResponse = false; throw new Error("Response lost after creation"); }
      return Response.json(result, { status: 201 });
    }
    const commentMatch = /\/issues\/(\d+)\/comments$/.exec(url.pathname);
    if (commentMatch && init.method === "POST") {
      const number = Number(commentMatch[1]);
      const result = { id: 1000 + this.comments.length, issue: number, body: JSON.parse(init.body).body,
        html_url: `https://github.com/youmikk/X-Battery/issues/${number}#issuecomment-${1000 + this.comments.length}`,
        user: { login: "relay-bot" }, author_association: "COLLABORATOR", created_at: "2026-11-23T01:00:00Z" };
      this.comments.push(result);
      if (this.loseCommentResponse) { this.loseCommentResponse = false; throw new Error("Comment response lost"); }
      return Response.json(result, { status: 201 });
    }
    if (commentMatch) {
      const page = Number(url.searchParams.get("page"));
      const rows = this.comments.filter(row => row.issue === Number(commentMatch[1]));
      return Response.json(rows.slice((page - 1) * 100, page * 100));
    }
    if (url.pathname.endsWith("/issues")) return Response.json([...this.issues].reverse());
    const issueMatch = /\/issues\/(\d+)$/.exec(url.pathname);
    if (issueMatch) {
      const row = this.issues.find(issue => issue.number === Number(issueMatch[1]));
      return row ? Response.json({ ...row, comments: this.comments.filter(comment => comment.issue === row.number).length })
        : new Response("{}", { status: 404 });
    }
    throw new Error(`Unexpected test request ${input}`);
  };
}

function body(changes = {}) {
  return { schema: 1, requestId: UUID, replyKey: KEY, category: "battery", appVersion: "1.4.9-beta.1",
    device: { name: "示例鼠标", family: "mouse", connection: "2.4GHz", brand: "示例品牌", model: "M1" },
    description: "电量更新后出现未知，请检查。", ...changes };
}

function setup() {
  const storage = new MemoryStorage();
  const bucket = new MemoryR2();
  const github = new FakeGithub();
  const env = { PUBLIC_BASE_URL: "https://feedback.example.org", GITHUB_TOKEN: "fake-test-token",
    GITHUB_AUTHOR_LOGIN: "relay-bot", FEEDBACK_SECRET: "71".repeat(32), ATTACHMENTS: bucket };
  let now = BASE_TIME;
  const coordinator = new FeedbackCoordinator({ storage }, env, { fetch: github.fetch, now: () => now });
  env.FEEDBACK = { idFromName: name => name, get: () => ({ fetch: request => coordinator.fetch(request) }) };
  return { storage, bucket, github, env, coordinator, advance: ms => { now += ms; } };
}

function request(path, data, { key, ip = IP, method = data === undefined ? "GET" : "POST", headers = {} } = {}) {
  return new Request(`https://feedback.example.org${path}`, { method,
    headers: { "X-Relay-Ip": ip, ...(data === undefined ? {} : { "Content-Type": "application/json" }),
      ...(key ? { Authorization: `Bearer ${key}` } : {}), ...headers },
    body: data === undefined ? undefined : JSON.stringify(data) });
}

async function promptly(operation) {
  let timer;
  try {
    return await Promise.race([operation, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error("Independent operation was blocked")), 2000);
    })]);
  } finally { clearTimeout(timer); }
}

function gate(context) {
  const entered = Promise.withResolvers();
  const released = Promise.withResolvers();
  context.after(() => released.resolve());
  return { entered: entered.promise, release: released.resolve,
    wait: async () => { entered.resolve(); await released.promise; } };
}

function slowBody(context, ip = IP) {
  const reading = Promise.withResolvers();
  let controller;
  let closed = false;
  let cancelled = false;
  const stream = new ReadableStream({
    start(value) { controller = value; },
    pull() { reading.resolve(); },
    cancel() { closed = true; cancelled = true; },
  }, { highWaterMark: 0 });
  const abort = () => {
    if (!closed) { closed = true; controller.error(new Error("Test upload stopped")); }
  };
  context.after(abort);
  return { request: new Request("https://feedback.example.org/feedback", { method: "POST",
    headers: { "Content-Type": "application/json", "X-Relay-Ip": ip }, body: stream, duplex: "half" }),
    reading: reading.promise, enqueue: bytes => controller.enqueue(bytes), abort,
    get cancelled() { return cancelled; } };
}

function assertIdle(coordinator) {
  assert.equal(coordinator.activeRequests, 0);
  assert.equal(coordinator.activeByIp.size, 0);
  assert.equal(coordinator.bodyBytes, 0);
  assert.equal(coordinator.queues.size, 0);
}

test("binds platform fetch to the global receiver for submissions and replies", async () => {
  const state = setup();
  const platformFetch = async function (input, init) {
    assert.equal(this, globalThis);
    return state.github.fetch(input, init);
  };
  const coordinator = new FeedbackCoordinator({ storage: state.storage }, state.env,
    { fetch: platformFetch, now: () => BASE_TIME });
  const submitted = await coordinator.fetch(request("/feedback", body()));
  assert.equal(submitted.status, 201);
  assert.equal(state.github.issues.length, 1);
  const updates = await coordinator.fetch(request("/feedback/123", undefined, { key: KEY }));
  assert.equal(updates.status, 200);
});

test("does not follow GitHub redirects with service credentials", async () => {
  const requests = [];
  const github = new Github("unused-test-token", "youmikk", async function (input, init) {
    assert.equal(this, globalThis);
    assert.equal(init.redirect, "manual");
    requests.push(input);
    return new Response(null, { status: 307, headers: { Location: "https://redirect.invalid/" } });
  });
  await assert.rejects(github.request("/issues"), { code: "github_unavailable" });
  assert.deepEqual(requests, ["https://api.github.com/repos/youmikk/X-Battery/issues"]);
});

test("creates one fixed-repository issue; replay survives coordinator restart without leaking reply key", async () => {
  const s = setup();
  const value = body({ description: `出现错误 ${KEY} @maintainer <img src=x>`, diagnostics: `path=sample\nkey=${KEY}\n\`\`\`\n@everyone` });
  const result = await s.coordinator.fetch(request("/feedback", value));
  assert.equal(result.status, 201);
  const receipt = await result.json();
  assert.deepEqual(receipt, { number: 123, url: "https://github.com/youmikk/X-Battery/issues/123", replyAccess: true });
  const restarted = new FeedbackCoordinator({ storage: s.storage }, s.env, { fetch: s.github.fetch, now: () => BASE_TIME });
  const replay = await restarted.fetch(request("/feedback", value));
  assert.equal(replay.status, 200);
  assert.deepEqual(await replay.json(), receipt);
  assert.equal(s.github.calls.filter(row => row.method === "POST").length, 1);
  const issue = s.github.issues[0];
  assert.ok(issue.body.includes("应用版本：1\\.4\\.9\\-beta\\.1"));
  assert.ok(!issue.body.includes(KEY));
  assert.ok(!issue.body.includes("@maintainer"));
  assert.ok(!issue.body.includes("<img"));
  assert.ok(!JSON.stringify([...s.storage.data.values()]).includes(KEY));
});

test("concurrent equal requests produce one issue and conflicting content or credentials is rejected", async () => {
  const s = setup();
  const results = await Promise.all([s.coordinator.fetch(request("/feedback", body())), s.coordinator.fetch(request("/feedback", body()))]);
  assert.deepEqual(results.map(result => result.status), [201, 200]);
  assert.equal(s.github.issues.length, 1);
  for (const changed of [body({ description: "不同的问题描述" }), body({ replyKey: "ff".repeat(32) })]) {
    assert.equal((await s.coordinator.fetch(request("/feedback", changed))).status, 409);
  }
});

test("lost successful POST response reconciles via marker and does not repeat the write", async () => {
  const s = setup(); s.github.loseIssueResponse = true;
  assert.equal((await s.coordinator.fetch(request("/feedback", body()))).status, 202);
  s.advance(6000);
  const recovered = await s.coordinator.fetch(request("/feedback", body()));
  assert.equal(recovered.status, 200);
  assert.equal((await recovered.json()).number, 123);
  assert.equal(s.github.calls.filter(row => row.method === "POST").length, 1);
  assert.ok(await s.storage.get("issue:123"));
});

test("uncertain failure with no issue stays pending instead of risking duplicate creation", async () => {
  const s = setup(); s.github.noIssueCreated = true;
  assert.equal((await s.coordinator.fetch(request("/feedback", body()))).status, 202);
  s.advance(6000);
  assert.equal((await s.coordinator.fetch(request("/feedback", body()))).status, 202);
  s.advance(3600000);
  assert.equal((await s.coordinator.fetch(request("/feedback", body()))).status, 202);
  assert.equal(s.github.calls.filter(row => row.method === "POST").length, 1);
});

test("definite GitHub rejection is terminal for the original request", async () => {
  const s = setup(); s.github.reject = 403;
  assert.equal((await s.coordinator.fetch(request("/feedback", body()))).status, 502);
  assert.equal((await s.coordinator.fetch(request("/feedback", body()))).status, 502);
  assert.equal(s.github.calls.filter(row => row.method === "POST").length, 1);
});

test("replies need the matching key and are idempotent including interrupted success", async () => {
  const s = setup();
  await s.coordinator.fetch(request("/feedback", body()));
  const message = { schema: 1, requestId: "c94ea958-d7e7-4d62-b4a0-0cd896c99eb7", body: "补充测试结果。" };
  assert.equal((await s.coordinator.fetch(request("/feedback/123/replies", message))).status, 401);
  assert.equal((await s.coordinator.fetch(request("/feedback/123/replies", message, { key: "ff".repeat(32) }))).status, 403);
  s.github.loseCommentResponse = true;
  assert.equal((await s.coordinator.fetch(request("/feedback/123/replies", message, { key: KEY }))).status, 202);
  s.advance(6000);
  const recovered = await s.coordinator.fetch(request("/feedback/123/replies", message, { key: KEY }));
  assert.equal(recovered.status, 200);
  assert.deepEqual(await recovered.json(), { id: 1000, url: "https://github.com/youmikk/X-Battery/issues/123#issuecomment-1000", createdAt: "2026-11-23T01:00:00Z" });
  assert.equal(s.github.comments.length, 1);
  assert.equal((await s.coordinator.fetch(request("/feedback/123/replies", { ...message, body: "changed" }, { key: KEY }))).status, 409);
});

test("thread is authenticated, returns only maintainer comments and hides relay metadata", async () => {
  const s = setup(); await s.coordinator.fetch(request("/feedback", body()));
  assert.equal((await s.coordinator.fetch(request("/feedback/123"))).status, 401);
  assert.equal((await s.coordinator.fetch(request("/feedback/123", undefined, { key: "ff".repeat(32) }))).status, 403);
  for (const [association, author, text] of [["OWNER", "youmikk", "已加入测试版。"], ["NONE", "other", "Not a maintainer"],
    ["COLLABORATOR", "relay-bot", `转发回复 <!-- r-battery-reply:${"a".repeat(64)} -->`]]) {
    const id = 2000 + s.github.comments.length;
    s.github.comments.push({ id, issue: 123, author_association: association, user: { login: author }, body: text,
      html_url: `https://github.com/youmikk/X-Battery/issues/123#issuecomment-${id}`, created_at: "2026-11-23T02:00:00Z" });
  }
  const response = await s.coordinator.fetch(request("/feedback/123", undefined, { key: KEY }));
  assert.equal(response.status, 200);
  const value = await response.json();
  assert.equal(value.comments.length, 1);
  assert.equal(value.comments[0].author, "youmikk");
  assert.equal(value.comments[0].isMaintainer, true);
  assert.equal(value.commentsTruncated, false);
  assert.ok(!value.issue.body.includes("r-battery-feedback:"));
  const before = s.github.calls.length;
  await s.coordinator.fetch(request("/feedback/123", undefined, { key: KEY }));
  assert.equal(s.github.calls.length, before);
});

test("image storage uses random server paths; R2-backed URL returns a safe content type", async () => {
  const s = setup();
  const result = await s.coordinator.fetch(request("/feedback", body({ images: [{ name: "截图.png", mimeType: "image/png", dataBase64: PNG }] })));
  assert.equal(result.status, 201);
  assert.equal(s.bucket.puts, 1);
  const [key] = s.bucket.data.keys();
  assert.match(key, /^attachments\/[a-f0-9]{48}$/);
  assert.ok(s.github.issues[0].body.includes(`https://feedback.example.org/${key}`));
  const image = await worker.fetch(new Request(`https://feedback.example.org/${key}`), s.env);
  assert.equal(image.status, 200);
  assert.equal(image.headers.get("Content-Type"), "image/png");
  assert.equal(image.headers.get("X-Content-Type-Options"), "nosniff");
  assert.ok(image.headers.get("Content-Security-Policy").includes("sandbox"));
  assert.deepEqual(new Uint8Array(await image.arrayBuffer()), Uint8Array.from(atob(PNG), char => char.charCodeAt(0)));
  const traversal = await worker.fetch(new Request("https://feedback.example.org/attachments/%2e%2e%2fsecret"), s.env);
  assert.equal(traversal.status, 404);
});

test("malformed fields, dangerous filenames, spoofed image formats and excessive sizes fail before writes", async () => {
  const s = setup();
  const invalid = [body({ schema: 2 }), body({ category: "arbitrary" }), body({ appVersion: "1.4 / test" }),
    body({ requestId: "not-uuid" }), body({ repo: "attacker/repo" }), body({ description: "bad" }),
    body({ diagnostics: "中".repeat(7000) }), body({ device: { ...body().device, family: "unknown" } }),
    body({ images: [{ name: "../x.png", mimeType: "image/png", dataBase64: PNG }] }),
    body({ images: [{ name: "x.svg", mimeType: "image/svg+xml", dataBase64: btoa("<svg/>") }] }),
    body({ images: [{ name: "x.png", mimeType: "image/png", dataBase64: btoa("<html>fake</html>") }] }),
    body({ images: [{ name: "x.png", mimeType: "image/png", dataBase64: PNG.slice(0, -1) + "!" }] }),
    body({ images: Array.from({ length: 4 }, () => ({ name: "x.png", mimeType: "image/png", dataBase64: PNG })) }),
    body({ images: [{ name: "large.png", mimeType: "image/png", dataBase64: Buffer.alloc(MAX_IMAGE_BYTES + 1).toString("base64") }] }),
  ];
  for (const value of invalid) assert.equal((await s.coordinator.fetch(request("/feedback", value))).status, 400);
  assert.equal(s.github.calls.length, 0);
  assert.equal(s.bucket.puts, 0);
});

test("body reader caps streamed bytes and rejects compression, wrong content type and invalid UTF8", async () => {
  const s = setup();
  for (const [headers, status] of [[{ "Content-Length": String(MAX_REQUEST_BYTES + 1) }, 413],
    [{ "Content-Encoding": "gzip" }, 415], [{ "Content-Type": "text/plain" }, 415]]) {
    assert.equal((await s.coordinator.fetch(request("/feedback", body(), { headers }))).status, status);
  }
  await assert.rejects(readJson(new Request("https://example.org", { method: "POST", headers: { "Content-Type": "application/json" },
    body: "1234567" }), 5), error => error.status === 413);
  await assert.rejects(readJson(new Request("https://example.org", { method: "POST", headers: { "Content-Type": "application/json" },
    body: new Uint8Array([0xff]) })), error => error.status === 400);
});

test("anonymous rate limits reject new issues but do not consume issue quota on replay", async () => {
  const s = setup();
  const original = body();
  await s.coordinator.fetch(request("/feedback", original));
  for (let n = 0; n < 4; n++) assert.equal((await s.coordinator.fetch(request("/feedback", original))).status, 200);
  for (const index of [1, 2]) {
    const value = body({ requestId: `45a526bf-b747-4d42-81e8-d1036d01f98${index}` });
    assert.equal((await s.coordinator.fetch(request("/feedback", value))).status, 201);
  }
  const limited = await s.coordinator.fetch(request("/feedback", body({ requestId: "45a526bf-b747-4d42-81e8-d1036d01f989" })));
  assert.equal(limited.status, 429);
  assert.equal((await limited.json()).error.code, "rate_limited");
  assert.equal(s.github.issues.length, 3);
});

test("public boundary overwrites forged rate identity and disabled config returns a useful error", async () => {
  const s = setup();
  const sent = request("/feedback", body(), { headers: { "CF-Connecting-IP": "192.0.2.8", "X-Relay-Ip": "fake" } });
  assert.equal((await worker.fetch(sent, s.env)).status, 201);
  assert.equal((await worker.fetch(request("/feedback", body()), s.env)).status, 503);
  const disabled = await worker.fetch(request("/feedback", body()), { ...s.env, PUBLIC_BASE_URL: "" });
  assert.equal(disabled.status, 503);
  assert.equal((await disabled.json()).error.code, "not_configured");
});

test("GitHub issue and comment URLs cannot escape the fixed repository", () => {
  assert.equal(checkGithubUrl("https://github.com/youmikk/X-Battery/issues/123", 123), "https://github.com/youmikk/X-Battery/issues/123");
  for (const url of ["https://github.com/evil/repo/issues/123", "https://github.com.evil.org/youmikk/X-Battery/issues/123",
    "https://user@github.com/youmikk/X-Battery/issues/123", "https://github.com/youmikk/X-Battery/issues/123?x=y",
    "https://github.com/youmikk/X-Battery/issues/123#issuecomment-9"]) assert.throws(() => checkGithubUrl(url, 123));
  assert.throws(() => checkGithubUrl("https://github.com/youmikk/X-Battery/issues/123#issuecomment-9", 123, 8));
});

test("public recovery marker copied by a different account cannot claim a feedback record", async () => {
  const s = setup(); s.github.noIssueCreated = true;
  await s.coordinator.fetch(request("/feedback", body()));
  const pending = await s.storage.get(`request:${UUID}`);
  s.github.issues.push({ number: 999, html_url: "https://github.com/youmikk/X-Battery/issues/999",
    body: pending.marker, user: { login: "attacker" }, created_at: "2026-11-23T00:00:00Z" });
  s.advance(6000);
  assert.equal((await s.coordinator.fetch(request("/feedback", body()))).status, 202);
  assert.equal(await s.storage.get("issue:999"), undefined);
});

test("schema examples and reply body validate without requiring diagnostics or screenshots", async () => {
  for (const family of ["mouse", "keyboard", "headset", "gamepad", "other"]) {
    const normalized = await feedback(body({ device: { name: "Example", family, connection: "unknown" }, appVersion: "1.4.9" }));
    assert.equal(normalized.device.family, family);
    assert.equal(normalized.images.length, 0);
  }
  assert.equal((await reply({ schema: 1, requestId: UUID, body: "一" })).body, "一");
});

test("alarm recovers an interrupted issue and schedules its next run", async () => {
  const s = setup(); s.github.loseIssueResponse = true;
  await s.coordinator.fetch(request("/feedback", body()));
  s.advance(6000);
  await s.coordinator.alarm();
  assert.equal((await s.storage.get(`request:${UUID}`)).state, "complete");
  assert.ok(s.storage.scheduled > BASE_TIME + 6000);
  assert.equal(s.github.calls.filter(row => row.method === "POST").length, 1);
});

test("failed image upload can resume before any GitHub write with the original request", async () => {
  const s = setup();
  const realPut = s.bucket.put.bind(s.bucket);
  let fail = true;
  s.bucket.put = async (...args) => { if (fail) { fail = false; throw new Error("R2 unavailable"); } return realPut(...args); };
  const value = body({ images: [{ name: "screenshot.png", mimeType: "image/png", dataBase64: PNG }] });
  assert.equal((await s.coordinator.fetch(request("/feedback", value))).status, 503);
  assert.equal(s.github.calls.length, 0);
  const record = await s.storage.get(`request:${UUID}`);
  assert.equal(record.state, "uploading");
  assert.equal((await s.coordinator.fetch(request("/feedback", value))).status, 201);
  assert.equal(s.bucket.data.size, 1);
  assert.ok(s.bucket.data.has(`attachments/${record.attachments[0].id}`));
});

test("JPEG and WebP signatures validate and oversized pixel dimensions fail", async () => {
  const jpeg = Uint8Array.from([0xff, 0xd8, 0xff, 0xc0, 0, 8, 8, 0, 1, 0, 1, 1, 0xff, 0xd9]);
  const webp = new Uint8Array(30);
  const view = new DataView(webp.buffer);
  view.setUint32(0, 0x52494646); view.setUint32(4, 22, true); view.setUint32(8, 0x57454250);
  view.setUint32(12, 0x56503858); view.setUint32(16, 10, true);
  for (const [mimeType, bytes] of [["image/jpeg", jpeg], ["image/webp", webp]]) {
    assert.equal((await feedback(body({ images: [{ name: "image", mimeType, dataBase64: Buffer.from(bytes).toString("base64") }] }))).images.length, 1);
  }
  const png = Buffer.from(PNG, "base64"); png.writeUInt32BE(100000, 16);
  await assert.rejects(feedback(body({ images: [{ name: "large.png", mimeType: "image/png", dataBase64: png.toString("base64") }] })), /dimensions/);
});

test("large maintainer history is bounded below client response limit", async () => {
  const s = setup(); await s.coordinator.fetch(request("/feedback", body()));
  for (let index = 0; index < 10; index++) {
    const id = index + 100;
    s.github.comments.push({ id, issue: 123, body: "维".repeat(30000), user: { login: "youmikk" },
      author_association: "OWNER", created_at: "2026-11-23T02:00:00Z",
      html_url: `https://github.com/youmikk/X-Battery/issues/123#issuecomment-${id}` });
  }
  const response = await s.coordinator.fetch(request("/feedback/123", undefined, { key: KEY }));
  assert.equal(response.status, 200);
  const text = await response.text();
  assert.ok(Buffer.byteLength(text) <= 500 * 1024);
  assert.equal(JSON.parse(text).commentsTruncated, true);
  assert.equal((await s.storage.get("issue:123")).cache, undefined);
});

test("three maximum-size decoded images fit the streaming limit without persisting base64", async () => {
  const s = setup();
  const payload = Buffer.alloc(MAX_IMAGE_BYTES);
  Buffer.from(PNG, "base64").copy(payload);
  // Size boundary fixture; format checks use the PNG header, not full decoding.
  const dataBase64 = payload.toString("base64");
  const value = body({ images: [0, 1, 2].map(index => ({ name: `capture-${index}.png`, mimeType: "image/png", dataBase64 })) });
  assert.ok(Buffer.byteLength(JSON.stringify(value)) < MAX_REQUEST_BYTES);
  const result = await s.coordinator.fetch(request("/feedback", value));
  assert.equal(result.status, 201);
  assert.equal(s.bucket.data.size, 3);
  for (const record of s.storage.data.values()) assert.ok(Buffer.byteLength(JSON.stringify(record)) < 128 * 1024);
  const journal = await s.storage.get(`request:${UUID}`);
  assert.equal(journal.body, undefined);
  assert.equal(journal.title, undefined);
});

test("retry after the GitHub write but before durable completion recovers the existing issue", async () => {
  const s = setup();
  const realPut = s.storage.put.bind(s.storage);
  let fail = true;
  s.storage.put = async (key, value) => {
    if (key === "issue:123" && fail) { fail = false; throw new Error("Storage completion failed"); }
    return realPut(key, value);
  };
  assert.equal((await s.coordinator.fetch(request("/feedback", body()))).status, 202);
  s.advance(6000);
  const retry = await s.coordinator.fetch(request("/feedback", body()));
  assert.equal(retry.status, 200);
  assert.equal(s.github.issues.length, 1);
  assert.equal(s.github.calls.filter(row => row.method === "POST").length, 1);
});

test("rejected attachment cleanup retains a small idempotency tombstone", async () => {
  const s = setup(); s.github.reject = 422;
  const value = body({ images: [{ name: "capture.png", mimeType: "image/png", dataBase64: PNG }] });
  assert.equal((await s.coordinator.fetch(request("/feedback", value))).status, 502);
  assert.equal(s.bucket.data.size, 1);
  s.advance(25 * 3600000);
  await s.coordinator.alarm();
  assert.equal(s.bucket.data.size, 0);
  const record = await s.storage.get(`request:${UUID}`);
  assert.equal(record.state, "rejected");
  assert.equal(record.body, undefined);
  assert.equal((await s.coordinator.fetch(request("/feedback", value))).status, 502);
  assert.equal(s.github.calls.filter(row => row.method === "POST").length, 1);
});

test("a stalled body has a deadline so it cannot block the singleton indefinitely", async () => {
  let cancelled = false;
  const stream = new ReadableStream({ start() {}, cancel() { cancelled = true; } });
  const stalled = new Request("https://example.org", { method: "POST", headers: { "Content-Type": "application/json" }, body: stream, duplex: "half" });
  await assert.rejects(readJson(stalled, 1000, 5), error => error.status === 408 && error.retryable);
  assert.equal(cancelled, true);
});

test("a stalled upload does not block another IP's submission, authentication or thread", async context => {
  const state = setup();
  const slow = slowBody(context);
  const uploading = state.coordinator.fetch(slow.request);
  await promptly(slow.reading);
  const otherIp = "cd".repeat(32);
  const created = await promptly(state.coordinator.fetch(request("/feedback", body(), { ip: otherIp })));
  assert.equal(created.status, 201);
  assert.equal((await promptly(state.coordinator.fetch(request("/feedback/123", undefined,
    { key: "ff".repeat(32), ip: otherIp })))).status, 403);
  assert.equal((await promptly(state.coordinator.fetch(request("/feedback/123", undefined,
    { key: KEY, ip: otherIp })))).status, 200);
  slow.abort();
  assert.equal((await promptly(uploading)).status, 400);
  assertIdle(state.coordinator);
});

test("slow R2 upload isolates other feedback and alarm cannot delete active attachments", async context => {
  const state = setup();
  const upload = gate(context);
  const put = state.bucket.put.bind(state.bucket);
  state.bucket.put = async (...args) => { await put(...args); await upload.wait(); };
  const value = body({ images: [{ name: "capture.png", mimeType: "image/png", dataBase64: PNG }] });
  const sending = state.coordinator.fetch(request("/feedback", value));
  await promptly(upload.entered);
  state.advance(25 * 3600000);
  assert.equal((await promptly(state.coordinator.fetch(request("/feedback",
    body({ requestId: "45a526bf-b747-4d42-81e8-d1036d01f984" }), { ip: "cd".repeat(32) })))).status, 201);
  await promptly(state.coordinator.alarm());
  assert.equal((await state.storage.get(`request:${UUID}`)).state, "uploading");
  assert.equal(state.bucket.data.size, 1);
  upload.release();
  assert.equal((await promptly(sending)).status, 201);
  assert.equal((await state.storage.get(`request:${UUID}`)).state, "complete");
  assert.equal(state.bucket.puts, 1);
  assertIdle(state.coordinator);
});

test("slow issue POST preserves concurrent idempotency without blocking other issues or alarm", async context => {
  const state = setup();
  const posted = gate(context);
  const fetch = state.coordinator.github.fetch;
  state.coordinator.github.fetch = async (input, init) => {
    const result = await fetch(input, init);
    if (init.method === "POST" && JSON.parse(init.body).body.includes("等待响应")) await posted.wait();
    return result;
  };
  const value = body({ description: "等待响应期间的重复提交测试。" });
  const first = state.coordinator.fetch(request("/feedback", value));
  await promptly(posted.entered);
  const duplicate = state.coordinator.fetch(request("/feedback", value, { ip: "cd".repeat(32) }));
  const conflict = state.coordinator.fetch(request("/feedback", { ...value, replyKey: "ff".repeat(32) },
    { ip: "ef".repeat(32) }));
  assert.equal((await promptly(state.coordinator.fetch(request("/feedback",
    body({ requestId: "45a526bf-b747-4d42-81e8-d1036d01f984" }), { ip: "12".repeat(32) })))).status, 201);
  state.advance(6000);
  await promptly(state.coordinator.alarm());
  assert.equal((await state.storage.get(`request:${UUID}`)).state, "sending");
  assert.equal(state.github.calls.filter(call => call.method === "GET").length, 0);
  posted.release();
  const results = await promptly(Promise.all([first, duplicate, conflict]));
  assert.deepEqual(results.map(result => result.status), [201, 200, 409]);
  assert.deepEqual(await results[0].json(), await results[1].json());
  assert.equal(state.github.calls.filter(call => call.method === "POST").length, 2);
  assert.equal((await state.storage.get(`request:${UUID}`)).state, "complete");
  assertIdle(state.coordinator);
});

test("slow thread reads isolate other issues and queued reads reuse the fresh cache", async context => {
  const state = setup();
  await state.coordinator.fetch(request("/feedback", body()));
  await state.coordinator.fetch(request("/feedback", body({ requestId: "45a526bf-b747-4d42-81e8-d1036d01f984" })));
  const reading = gate(context);
  const fetch = state.coordinator.github.fetch;
  state.coordinator.github.fetch = async (input, init) => {
    if (new URL(input).pathname.endsWith("/issues/123")) await reading.wait();
    return fetch(input, init);
  };
  const first = state.coordinator.fetch(request("/feedback/123", undefined, { key: KEY }));
  await promptly(reading.entered);
  const second = state.coordinator.fetch(request("/feedback/123", undefined, { key: KEY, ip: "cd".repeat(32) }));
  assert.equal((await promptly(state.coordinator.fetch(request("/feedback/124", undefined,
    { key: KEY, ip: "ef".repeat(32) })))).status, 200);
  reading.release();
  const results = await promptly(Promise.all([first, second]));
  assert.deepEqual(results.map(result => result.status), [200, 200]);
  assert.deepEqual(await results[0].json(), await results[1].json());
  assert.equal(state.github.calls.filter(call => new URL(call.url).pathname.endsWith("/issues/123")).length, 1);
  assertIdle(state.coordinator);
});

test("concurrent replies keep one POST and refresh a queued thread after invalidating its cache", async context => {
  const state = setup();
  await state.coordinator.fetch(request("/feedback", body()));
  await state.coordinator.fetch(request("/feedback", body({ requestId: "45a526bf-b747-4d42-81e8-d1036d01f984" })));
  await state.coordinator.fetch(request("/feedback/123", undefined, { key: KEY }));
  const posted = gate(context);
  const fetch = state.coordinator.github.fetch;
  state.coordinator.github.fetch = async (input, init) => {
    const result = await fetch(input, init);
    if (init.method === "POST" && new URL(input).pathname.endsWith("/issues/123/comments")) await posted.wait();
    return result;
  };
  const message = { schema: 1, requestId: UUID, body: "补充测试结果。" };
  const first = state.coordinator.fetch(request("/feedback/123/replies", message, { key: KEY }));
  await promptly(posted.entered);
  const duplicate = state.coordinator.fetch(request("/feedback/123/replies", message,
    { key: KEY, ip: "cd".repeat(32) }));
  const updates = state.coordinator.fetch(request("/feedback/123", undefined, { key: KEY, ip: "ef".repeat(32) }));
  assert.equal((await promptly(state.coordinator.fetch(request("/feedback/124/replies", message,
    { key: KEY, ip: "12".repeat(32) })))).status, 201);
  state.advance(6000);
  await promptly(state.coordinator.alarm());
  assert.equal((await state.storage.get(`reply:123:${UUID}`)).state, "sending");
  posted.release();
  const results = await promptly(Promise.all([first, duplicate, updates]));
  assert.deepEqual(results.map(result => result.status), [201, 200, 200]);
  assert.deepEqual(await results[0].json(), await results[1].json());
  assert.equal(state.github.calls.filter(call => call.method === "POST" && call.url.endsWith("/issues/123/comments")).length, 1);
  assert.equal(state.github.calls.filter(call => call.method === "GET" && call.url.endsWith("/issues/123")).length, 2);
  assert.equal((await state.coordinator.fetch(request("/feedback/123/replies", { ...message, body: "不同补充" },
    { key: KEY }))).status, 409);
  assertIdle(state.coordinator);
});

test("concurrent IPs cannot lose quota increments or exceed the final global issue slot", async () => {
  const state = setup();
  const bucket = Math.floor(BASE_TIME / 86400000);
  await state.storage.put(`rate:new:global:${bucket}`, { count: 199, expiresAt: (bucket + 2) * 86400000 });
  const results = await Promise.all(Array.from({ length: 8 }, (_, index) => state.coordinator.fetch(request("/feedback",
    body({ requestId: `45a526bf-b747-4d42-81e8-d1036d01f98${index}` }), { ip: index.toString(16).repeat(64) }))));
  assert.equal(results.filter(result => result.status === 201).length, 1);
  assert.equal(results.filter(result => result.status === 429).length, 7);
  assert.equal((await state.storage.get(`rate:new:global:${bucket}`)).count, 200);
  assert.equal((await state.storage.get(`rate:requests:global:${bucket}`)).count, 8);
  assert.equal(state.github.issues.length, 1);
  assertIdle(state.coordinator);
});

test("admission caps per-IP and total in-flight requests and releases slots after failure", async context => {
  const state = setup();
  const uploads = [];
  const responses = [];
  for (let index = 0; index < 16; index++) {
    const upload = slowBody(context, Math.floor(index / 2).toString(16).repeat(64));
    uploads.push(upload);
    responses.push(state.coordinator.fetch(upload.request));
    await promptly(upload.reading);
  }
  const sameIp = slowBody(context, "0".repeat(64));
  const limited = await promptly(state.coordinator.fetch(sameIp.request));
  assert.equal(limited.status, 429);
  assert.equal(limited.headers.get("Retry-After"), "5");
  assert.equal(sameIp.cancelled, true);
  const otherIp = slowBody(context, IP);
  const busy = await promptly(state.coordinator.fetch(otherIp.request));
  assert.equal(busy.status, 503);
  assert.deepEqual((await busy.json()).error.code, "service_busy");
  assert.equal(busy.headers.get("Retry-After"), "5");
  assert.equal(otherIp.cancelled, true);
  for (const upload of uploads) upload.abort();
  assert.ok((await promptly(Promise.all(responses))).every(response => response.status === 400));
  assertIdle(state.coordinator);
  assert.equal((await state.coordinator.fetch(request("/feedback", body()))).status, 201);
  assertIdle(state.coordinator);
});

test("aggregate body budget cancels excess uploads and releases their partial reservation", async context => {
  const state = setup();
  const half = MAX_REQUEST_BYTES / 2;
  const first = slowBody(context);
  first.enqueue(Buffer.alloc(half + 1, 32));
  const uploading = state.coordinator.fetch(first.request);
  await promptly(first.reading);
  assert.equal(state.coordinator.bodyBytes, half + 1);
  const second = slowBody(context, "cd".repeat(32));
  second.enqueue(Buffer.alloc(half - 1, 32));
  const rejected = state.coordinator.fetch(second.request);
  await promptly(second.reading);
  assert.equal(state.coordinator.bodyBytes, MAX_REQUEST_BYTES);
  second.enqueue(new Uint8Array([32]));
  const result = await promptly(rejected);
  assert.equal(result.status, 503);
  assert.equal((await result.json()).error.code, "service_busy");
  assert.equal(second.cancelled, true);
  assert.equal(state.coordinator.bodyBytes, half + 1);
  assert.equal((await promptly(state.coordinator.fetch(request("/feedback", body(),
    { ip: "ef".repeat(32) })))).status, 201);
  first.abort();
  await promptly(uploading);
  assertIdle(state.coordinator);
});

test("body reservations remain held after parsing until attachment processing ends", async context => {
  const state = setup();
  const upload = gate(context);
  const put = state.bucket.put.bind(state.bucket);
  state.bucket.put = async (...args) => { await put(...args); await upload.wait(); };
  const value = body({ images: [{ name: "capture.png", mimeType: "image/png", dataBase64: PNG }] });
  const padded = data => JSON.stringify(data).padEnd(MAX_REQUEST_BYTES / 2 + 1, " ");
  const send = (data, ip) => state.coordinator.fetch(new Request("https://feedback.example.org/feedback", {
    method: "POST", headers: { "Content-Type": "application/json", "X-Relay-Ip": ip }, body: padded(data),
  }));
  const first = send(value, IP);
  await promptly(upload.entered);
  const reserved = state.coordinator.bodyBytes;
  assert.equal(reserved, Buffer.byteLength(padded(value)));
  const other = body({ requestId: "45a526bf-b747-4d42-81e8-d1036d01f984" });
  assert.equal((await promptly(send(other, "cd".repeat(32)))).status, 503);
  assert.equal(state.coordinator.bodyBytes, reserved);
  upload.release();
  assert.equal((await promptly(first)).status, 201);
  assertIdle(state.coordinator);
  assert.equal((await promptly(send(other, "cd".repeat(32)))).status, 201);
  assertIdle(state.coordinator);
});

test("failed quota storage, validation and upstream calls release all admission state", async () => {
  const state = setup();
  const put = state.storage.put.bind(state.storage);
  state.storage.put = async () => { throw new Error("Test storage unavailable"); };
  assert.equal((await state.coordinator.fetch(request("/feedback", body()))).status, 503);
  assertIdle(state.coordinator);
  state.storage.put = put;
  assert.equal((await state.coordinator.fetch(request("/feedback", body({ description: "bad" })))).status, 400);
  assertIdle(state.coordinator);
  state.github.noIssueCreated = true;
  assert.equal((await state.coordinator.fetch(request("/feedback", body()))).status, 202);
  assertIdle(state.coordinator);
  state.github.noIssueCreated = false;
  assert.equal((await state.coordinator.fetch(request("/feedback",
    body({ requestId: "45a526bf-b747-4d42-81e8-d1036d01f984" })))).status, 201);
  assertIdle(state.coordinator);
});

test("alarm recovery and client retry share the journal lock while other feedback progresses", async context => {
  const state = setup();
  state.github.loseIssueResponse = true;
  assert.equal((await state.coordinator.fetch(request("/feedback", body()))).status, 202);
  state.advance(6000);
  const recovering = gate(context);
  const fetch = state.coordinator.github.fetch;
  state.coordinator.github.fetch = async (input, init) => {
    if (init.method === "GET" && new URL(input).pathname.endsWith("/issues")) await recovering.wait();
    return fetch(input, init);
  };
  const alarm = state.coordinator.alarm();
  await promptly(recovering.entered);
  const retry = state.coordinator.fetch(request("/feedback", body()));
  assert.equal((await promptly(state.coordinator.fetch(request("/feedback",
    body({ requestId: "45a526bf-b747-4d42-81e8-d1036d01f984" }), { ip: "cd".repeat(32) })))).status, 201);
  recovering.release();
  await promptly(alarm);
  assert.equal((await promptly(retry)).status, 200);
  assert.equal((await state.storage.get(`request:${UUID}`)).state, "complete");
  assert.equal(state.github.calls.filter(call => call.method === "POST").length, 2);
  assert.equal(state.github.calls.filter(call => call.method === "GET").length, 1);
  assertIdle(state.coordinator);
});

test("alarm rereads a stale scan after the client has completed recovery", async context => {
  const state = setup();
  state.github.loseIssueResponse = true;
  await state.coordinator.fetch(request("/feedback", body()));
  state.advance(6000);
  const scanned = gate(context);
  const list = state.storage.list.bind(state.storage);
  state.storage.list = async options => {
    const rows = await list(options);
    if (options.prefix === "request:") await scanned.wait();
    return rows;
  };
  const alarm = state.coordinator.alarm();
  await promptly(scanned.entered);
  assert.equal((await promptly(state.coordinator.fetch(request("/feedback", body())))).status, 200);
  const calls = state.github.calls.length;
  scanned.release();
  await promptly(alarm);
  assert.equal(state.github.calls.length, calls);
  assert.equal((await state.storage.get(`request:${UUID}`)).state, "complete");
  assertIdle(state.coordinator);
});

test("alarm reply recovery and concurrent replay produce no second comment POST", async context => {
  const state = setup();
  await state.coordinator.fetch(request("/feedback", body()));
  state.github.loseCommentResponse = true;
  const message = { schema: 1, requestId: UUID, body: "补充测试结果。" };
  assert.equal((await state.coordinator.fetch(request("/feedback/123/replies", message, { key: KEY }))).status, 202);
  state.advance(6000);
  const recovering = gate(context);
  const fetch = state.coordinator.github.fetch;
  state.coordinator.github.fetch = async (input, init) => {
    if (init.method === "GET" && new URL(input).pathname.endsWith("/issues/123/comments")) await recovering.wait();
    return fetch(input, init);
  };
  const alarm = state.coordinator.alarm();
  await promptly(recovering.entered);
  const retry = state.coordinator.fetch(request("/feedback/123/replies", message, { key: KEY }));
  assert.equal((await promptly(state.coordinator.fetch(request("/feedback",
    body({ requestId: "45a526bf-b747-4d42-81e8-d1036d01f984" }), { ip: "cd".repeat(32) })))).status, 201);
  recovering.release();
  await promptly(alarm);
  assert.equal((await promptly(retry)).status, 200);
  assert.equal((await state.storage.get(`reply:123:${UUID}`)).state, "complete");
  assert.equal(state.github.calls.filter(call => call.method === "POST" && call.url.endsWith("/comments")).length, 1);
  assert.equal(state.github.calls.filter(call => call.method === "GET" && new URL(call.url).pathname.endsWith("/comments")).length, 1);
  assertIdle(state.coordinator);
});

test("oversized input returns promptly even if stream cancellation never resolves", async () => {
  let cancelled = false;
  const stream = new ReadableStream({
    start(controller) { controller.enqueue(new Uint8Array(6)); },
    cancel() { cancelled = true; return new Promise(() => {}); },
  });
  const oversized = new Request("https://example.org", { method: "POST",
    headers: { "Content-Type": "application/json" }, body: stream, duplex: "half" });
  await assert.rejects(promptly(readJson(oversized, 5)), error => error.status === 413);
  assert.equal(cancelled, true);
});
