import http from 'node:http';
import { randomBytes, timingSafeEqual } from 'node:crypto';
import { readFile, writeFile, mkdir, rename, copyFile, stat } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const work = path.join(root, 'scratch', 'mock-panel');
const profile = path.join(work, 'profile');
const sourceExe = path.join(root, 'target', 'release', 'rog-mouse-ctl.exe');
const testExe = path.join(work, 'R-Battery-Mock.exe');
const fixture = path.join(work, 'devices.txt');
const port = Number(process.env.ROG_MOCK_PANEL_PORT || 48763);
const origin = `http://127.0.0.1:${port}`;
const token = randomBytes(32).toString('hex');
const sample = [
  { family: 'mouse', model: 'ROG 龙鳞 ACE', level: 78, charging: false, connected: true },
  { family: 'keyboard', model: 'ROG 夜魔', level: 96, charging: true, connected: true },
  { family: 'headset', model: 'ROG 棱镜 S', level: 24, charging: false, connected: true },
  { family: 'gamepad', model: 'ROG 雷切 PRO', level: 64, charging: false, connected: false },
];
let devices = sample;
let child = null;
let launching = false;
let lastExit = '';
let shuttingDown = false;

export function validateDevices(value) {
  if (!Array.isArray(value) || value.length > 32) throw new Error('最多支持 32 台模拟设备');
  return value.map(row => {
    if (!row || !['mouse', 'keyboard', 'headset', 'gamepad', 'unknown'].includes(row.family)) {
      throw new Error('设备类型无效');
    }
    if (typeof row.model !== 'string' || !row.model.trim() || row.model.length > 80 || /[|\r\n\x00-\x1f]/u.test(row.model)) {
      throw new Error('设备名称不能为空，且不能包含换行或竖线');
    }
    if (!Number.isInteger(row.level) || row.level < 0 || row.level > 100) throw new Error('电量范围是 0–100');
    if (typeof row.charging !== 'boolean' || typeof row.connected !== 'boolean') throw new Error('设备状态无效');
    return { family: row.family, model: row.model.trim(), level: row.level, charging: row.charging, connected: row.connected };
  });
}

export function fixtureText(rows) {
  return rows.map(r => [r.family, r.model, r.level, Number(r.charging), r.connected ? 'online' : 'offline'].join('|')).join('\n') + '\n';
}

async function saveDevices(rows) {
  // 请求串行处理，替换时不会让程序读到半个文件。
  await writeFile(fixture + '.tmp', fixtureText(rows), 'utf8');
  await rename(fixture + '.tmp', fixture);
  await writeFile(path.join(work, 'devices.json'), JSON.stringify(rows), 'utf8');
  devices = rows;
}

function running() { return child !== null && child.exitCode === null && child.signalCode === null; }
function status() { return { running: running(), launching, devices, lastExit }; }
function reply(res, code, value) {
  res.writeHead(code, { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store' });
  res.end(JSON.stringify(value));
}

async function stopApp() {
  const owned = child;
  if (!owned || !running()) { child = null; return; }
  const exited = new Promise(resolve => owned.once('exit', resolve));
  owned.kill();
  await exited;
  if (child === owned) child = null;
}

async function startApp(restart) {
  if (restart) await stopApp();
  if (running()) return;
  try { await stat(sourceExe); }
  catch { throw new Error('找不到 release 程序，请先构建 R Battery'); }
  await copyFile(sourceExe, testExe);
  launching = true;
  lastExit = '';
  const owned = spawn(testExe, [], {
    cwd: work, windowsHide: false, stdio: 'ignore',
    env: { ...process.env, ROG_MOCK_DEVICES: fixture, APPDATA: profile, LOCALAPPDATA: profile },
  });
  child = owned;
  owned.once('exit', code => {
    if (child === owned) child = null;
    lastExit = code === 0 ? '' : `测试窗口已退出（${code}）`;
  });
  try {
    await new Promise((resolve, reject) => {
      owned.once('spawn', resolve);
      owned.once('error', reject);
    });
    const ready = spawn('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
      path.join(root, 'tools', 'mock-devices-window.ps1'), '-ProcessId', String(owned.pid)],
      { windowsHide: true, stdio: 'ignore' });
    const result = await new Promise((resolve, reject) => {
      ready.once('exit', resolve);
      ready.once('error', reject);
    });
    if (result !== 0 || !running()) {
      await stopApp();
      throw new Error(lastExit || '测试窗口未能显示，请停止其他模拟实例后重试');
    }
  } finally { launching = false; }
}

async function body(req) {
  const chunks = [];
  let length = 0;
  for await (const chunk of req) {
    length += chunk.length;
    if (length > 32768) throw new Error('请求内容过大');
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}

let queue = Promise.resolve();
const server = http.createServer((req, res) => {
  if (req.headers.host !== `127.0.0.1:${port}` || (req.headers.origin && req.headers.origin !== origin)) {
    reply(res, 403, { error: '请求来源无效' }); return;
  }
  queue = queue.then(async () => {
    if (req.method === 'GET' && req.url === '/') {
      const html = (await readFile(path.join(root, 'tools', 'mock-devices.html'), 'utf8')).replace('__MOCK_PANEL_TOKEN__', token);
      res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store',
        'X-Frame-Options': 'DENY', 'X-Content-Type-Options': 'nosniff',
        'Content-Security-Policy': "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'" });
      res.end(html); return;
    }
    if (req.method === 'GET' && req.url === '/health') { reply(res, 200, { app: 'r-battery-mock-panel' }); return; }
    const provided = Buffer.from(String(req.headers['x-mock-token'] || ''));
    const expected = Buffer.from(token);
    if (provided.length !== expected.length || !timingSafeEqual(provided, expected)) {
      reply(res, 403, { error: '请重新打开本地测试面板' }); return;
    }
    if (req.method === 'GET' && req.url === '/api/state') { reply(res, 200, status()); return; }
    if (req.method !== 'POST' || req.headers['content-type']?.split(';')[0] !== 'application/json') {
      reply(res, 405, { error: '不支持的请求' }); return;
    }
    const input = await body(req);
    switch (req.url) {
      case '/api/devices': await saveDevices(validateDevices(input.devices)); break;
      case '/api/start': await startApp(false); break;
      case '/api/restart': await startApp(true); break;
      case '/api/stop': await stopApp(); break;
      case '/api/quit':
        reply(res, 200, { stopped: true });
        void shutdown(); return;
      default: reply(res, 404, { error: '接口不存在' }); return;
    }
    reply(res, 200, status());
  }).catch(error => { if (!res.headersSent) reply(res, 400, { error: error.message }); else res.end(); });
});
server.requestTimeout = 10000;
server.headersTimeout = 10000;
server.on('error', error => { console.error(error.message); process.exitCode = 1; });

async function shutdown() {
  if (shuttingDown) return;
  shuttingDown = true;
  await stopApp();
  server.close();
  server.closeAllConnections();
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await mkdir(path.join(profile, 'rog-mouse-ctl'), { recursive: true });
  try { devices = validateDevices(JSON.parse(await readFile(path.join(work, 'devices.json'), 'utf8'))); } catch {}
  const settings = path.join(profile, 'rog-mouse-ctl', 'settings.ini');
  try { await writeFile(settings, 'close_mode=quit\nrefresh_interval_sec=5\n', { flag: 'wx' }); } catch (e) { if (e.code !== 'EEXIST') throw e; }
  await saveDevices(devices);
  server.listen(port, '127.0.0.1', () => console.log(`R Battery mock panel: ${origin}`));
  process.once('SIGINT', shutdown);
  process.once('SIGTERM', shutdown);
}
