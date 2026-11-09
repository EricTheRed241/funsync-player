#!/usr/bin/env node
// VRBangers member downloads through the installed Google Chrome.
//
//   node scripts/vrbangers-browser.mjs --links <slug>      the scene's best download within the VR caps
//   node scripts/vrbangers-browser.mjs --public <slug>     what the public scene page says (no browser)
//
// vrbangers.com signs its download links per member, inside the scene page's
// `#__NUXT_DATA__` (scripts/vrbangers-lib.mjs picks the scene's own). Chrome
// runs headless with its own profile, `<userData>/vrbangers-chrome/`, so the
// login survives runs; it is driven over the DevTools protocol (Node's
// WebSocket, no extra dependency) and always closed on exit. The account comes
// from `vb_hack.txt` (repo root, untracked; `Username: …` / `password: …`).
// The password, cookies and link tokens are never printed.
//
// Exit codes: 0 done, 1 error, 3 not logged in (no credentials or the login failed).

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SELF = fileURLToPath(import.meta.url);
const IS_CLI = path.resolve(process.argv[1] || '') === SELF;

// renderer/*.js is ESM inside a CommonJS package. Re-exec once so Node does
// not print MODULE_TYPELESS_PACKAGE_JSON on every run.
if (IS_CLI && !process.execArgv.includes('--disable-warning=MODULE_TYPELESS_PACKAGE_JSON')) {
  const relaunch = spawnSync(process.execPath, [
    '--disable-warning=MODULE_TYPELESS_PACKAGE_JSON',
    ...process.argv.slice(1),
  ], { stdio: 'inherit' });
  process.exit(relaunch.status ?? 1);
}

const lib = await import('./vrbangers-lib.mjs');
const { createPacer, defaultUserDataDir } = await import('./eroscripts-sync-lib.mjs');

const REPO = path.dirname(path.dirname(SELF));
export const CHROME_PATH = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
export const DEFAULT_CREDENTIALS = path.join(REPO, 'vb_hack.txt');
export const PROFILE_DIR = 'vrbangers-chrome';
const PAGE_INTERVAL_MS = 1000;
const LOAD_TIMEOUT_MS = 60_000;
const LOGIN_TIMEOUT_MS = 30_000;

/** Not logged in: no credentials, or vrbangers.com refused them (CLI exit code 3). */
export class LoginError extends Error {
  constructor(message) { super(message); this.name = 'LoginError'; this.exitCode = 3; }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** The account in the credentials file; LoginError when missing or unreadable. */
export function readCredentials(file = DEFAULT_CREDENTIALS) {
  let text;
  try { text = fs.readFileSync(file, 'utf8'); } catch { throw new LoginError(`No VRBangers credentials file: ${file}`); }
  const creds = lib.parseCredentials(text);
  if (!creds) throw new LoginError(`No "Username:" / "password:" lines in ${file}`);
  return creds;
}

/** The public scene page, parsed (lib.parseScenePage); no login, no browser. */
export async function scenePublic(slug, { fetchImpl = fetch, userAgent = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36' } = {}) {
  const res = await fetchImpl(lib.sceneUrl(slug), { headers: { 'user-agent': userAgent, accept: 'text/html' } });
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`vrbangers.com ${res.status} for /video/${slug}/`);
  return lib.parseScenePage(await res.text(), slug);
}

// ── Chrome over the DevTools protocol ───────────────────────────────────

async function launchChrome({ chromePath, profile, headless }) {
  fs.mkdirSync(profile, { recursive: true });
  const portFile = path.join(profile, 'DevToolsActivePort');
  fs.rmSync(portFile, { force: true });
  const args = [
    `--user-data-dir=${profile}`, '--remote-debugging-port=0', '--no-first-run', '--no-default-browser-check',
    '--disable-background-networking', '--disable-sync', '--mute-audio', '--window-size=1366,900',
    ...(headless ? ['--headless=new'] : []), 'about:blank',
  ];
  const child = spawn(chromePath, args, { stdio: ['ignore', 'ignore', 'pipe'] });
  let stderr = '';
  child.stderr.on('data', (d) => { stderr = (stderr + d).slice(-2000); });
  let exited = null;
  child.on('exit', (code, signal) => { exited = { code, signal }; });
  child.on('error', (err) => { exited = { error: err }; });
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (exited) {
      throw new Error(`Chrome did not start (${exited.error?.message || `exit ${exited.code ?? exited.signal}`}); is another run using ${profile}?\n${stderr.trim()}`);
    }
    const lines = fs.existsSync(portFile) ? fs.readFileSync(portFile, 'utf8').split('\n') : [];
    if (lines[0] && lines[1]) return { child, wsUrl: `ws://127.0.0.1:${lines[0].trim()}${lines[1].trim()}` };
    await sleep(100);
  }
  child.kill('SIGKILL');
  throw new Error('Chrome did not open its DevTools port within 30 s');
}

function connect(wsUrl) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    let nextId = 1;
    const pending = new Map();
    const listeners = new Set();
    const conn = {
      send(method, params = {}, sessionId) {
        const id = nextId++;
        ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
        return new Promise((res, rej) => pending.set(id, { res, rej, method }));
      },
      /** Resolves with the first `method` event (of `sessionId`) that `test` accepts. */
      once(method, sessionId, test = () => true, timeoutMs = LOAD_TIMEOUT_MS) {
        return new Promise((res, rej) => {
          const entry = (msg) => {
            if (msg.method !== method || (sessionId && msg.sessionId !== sessionId) || !test(msg.params)) return;
            listeners.delete(entry); clearTimeout(timer); res(msg.params);
          };
          const timer = setTimeout(() => { listeners.delete(entry); rej(new Error(`Timed out waiting for ${method}`)); }, timeoutMs);
          listeners.add(entry);
        });
      },
      close() { try { ws.close(); } catch { /* already closed */ } },
    };
    ws.addEventListener('open', () => resolve(conn));
    ws.addEventListener('error', () => reject(new Error(`DevTools connection failed: ${wsUrl}`)));
    ws.addEventListener('close', () => {
      for (const { rej, method } of pending.values()) rej(new Error(`Chrome closed during ${method}`));
      pending.clear();
    });
    ws.addEventListener('message', (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id && pending.has(msg.id)) {
        const { res, rej, method } = pending.get(msg.id);
        pending.delete(msg.id);
        if (msg.error) rej(new Error(`${method}: ${msg.error.message}`)); else res(msg.result);
        return;
      }
      for (const listener of [...listeners]) listener(msg);
    });
  });
}

/**
 * Open a VRBangers session in a headless Chrome. Call `close()` when done
 * (also done on process exit / SIGINT / SIGTERM).
 *
 * @returns {Promise<{ ensureLoggedIn(): Promise<void>, sceneLinks(slug: string): Promise<object[]>,
 *   nuxtData(slug: string): Promise<any[]|null>, userAgent: string, close(): Promise<void> }>}
 */
export async function openVrbangers({
  userData = defaultUserDataDir(),
  credentialsFile = DEFAULT_CREDENTIALS,
  chromePath = CHROME_PATH,
  headless = true,
  log = () => {},
} = {}) {
  const profile = path.join(userData, PROFILE_DIR);
  const { child, wsUrl } = await launchChrome({ chromePath, profile, headless });
  const kill = () => { if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL'); };
  const onSignal = (signal) => { kill(); process.exit(signal === 'SIGINT' ? 130 : 143); };
  process.on('exit', kill);
  process.on('SIGINT', onSignal);
  process.on('SIGTERM', onSignal);
  let conn;
  let closed = false;
  const close = async () => {
    if (closed) return;
    closed = true;
    process.off('exit', kill);
    process.off('SIGINT', onSignal);
    process.off('SIGTERM', onSignal);
    if (conn) {
      await Promise.race([conn.send('Browser.close').catch(() => {}), sleep(3000)]);
      conn.close();
    }
    const exited = child.exitCode !== null || child.signalCode !== null
      ? Promise.resolve() : new Promise((resolve) => child.once('exit', resolve));
    await Promise.race([exited, sleep(5000)]);
    kill();
  };

  try {
    conn = await connect(wsUrl);
    const { userAgent: headlessUa } = await conn.send('Browser.getVersion');
    const userAgent = headlessUa.replace('HeadlessChrome', 'Chrome');
    const { targetId } = await conn.send('Target.createTarget', { url: 'about:blank' });
    const { sessionId } = await conn.send('Target.attachToTarget', { targetId, flatten: true });
    const page = (method, params) => conn.send(method, params, sessionId);
    await page('Page.enable');
    await page('Network.setUserAgentOverride', { userAgent });
    const pacer = createPacer({ intervalMs: PAGE_INTERVAL_MS });

    const evaluate = async (expression) => {
      const { result, exceptionDetails } = await page('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
      if (exceptionDetails) throw new Error(`Page script failed: ${exceptionDetails.exception?.description || exceptionDetails.text}`);
      return result.value;
    };
    const navigate = async (url) => {
      await pacer.wait();
      log(`open ${url}`);
      const loaded = conn.once('Page.loadEventFired', sessionId);
      const { errorText } = await page('Page.navigate', { url });
      if (errorText) throw new Error(`Could not open ${url}: ${errorText}`);
      await loaded;
    };
    const waitFor = async (expression, timeoutMs) => {
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        if (await evaluate(expression).catch(() => false)) return true;
        await sleep(250);
      }
      return false;
    };
    const onDashboard = `location.pathname.startsWith(${JSON.stringify(lib.DASHBOARD_PATH)})`;
    const hasLoginForm = `!!document.querySelector('input[name="mail"]')`;

    async function ensureLoggedIn() {
      await navigate(lib.LOGIN_URL);
      // A kept session goes on to the dashboard; the form shows up otherwise.
      await waitFor(`${onDashboard} || ${hasLoginForm}`, 15_000);
      if (await evaluate(`${onDashboard} || !(${hasLoginForm})`)) return;
      const { user, password } = readCredentials(credentialsFile);
      log('logging in');
      await evaluate(`(() => {
        for (const b of document.querySelectorAll('button, a')) {
          if (/^\\s*(I Agree|Accept)\\s*$/i.test(b.textContent || '')) b.click();
        }
        document.querySelectorAll('.splash-screen, .splash-screen__text-container').forEach((el) => el.remove());
      })()`);
      for (const [name, value] of [['mail', user], ['password', password]]) {
        await evaluate(`(() => { const el = document.querySelector('input[name="${name}"]'); el.focus(); el.select(); })()`);
        await page('Input.insertText', { text: value });
      }
      const filled = await evaluate(`[...document.querySelectorAll('input[name="mail"], input[name="password"]')].every((el) => el.value.length > 0)`);
      if (!filled) throw new LoginError('VRBangers login form did not take the credentials');
      await evaluate(`(() => {
        document.querySelectorAll('.splash-screen, .splash-screen__text-container').forEach((el) => el.remove());
        document.querySelector('button[type="submit"]').click();
      })()`);
      if (await waitFor(onDashboard, LOGIN_TIMEOUT_MS)) return;
      const shown = await evaluate(`(document.querySelector('.error, .form-error, [class*="error"]')?.textContent || '').trim().slice(0, 200)`).catch(() => '');
      throw new LoginError(`VRBangers login failed (still on ${await evaluate('location.pathname').catch(() => '?')})${shown ? `: ${shown}` : ''}`);
    }

    /** The scene page's raw `#__NUXT_DATA__` array (null when the page has none). */
    async function nuxtData(slug) {
      await navigate(lib.sceneUrl(slug));
      const text = await evaluate(`document.getElementById('__NUXT_DATA__')?.textContent || ''`);
      if (!text) return null;
      try { return JSON.parse(text); } catch { return null; }
    }

    /** This scene's signed downloads (lib.pickSceneLinks), highest quality first. */
    async function sceneLinks(slug) {
      return lib.pickSceneLinks(await nuxtData(slug) || [], slug);
    }

    return { ensureLoggedIn, sceneLinks, nuxtData, userAgent, close };
  } catch (err) {
    await close();
    throw err;
  }
}

// ── CLI ────────────────────────────────────────────────────────────────

const USAGE = `Usage: node scripts/vrbangers-browser.mjs <command> [options]
  --links <slug>         log in and print the scene's downloads and the one within the VR caps
  --public <slug>        print what the public scene page says (no login, no browser)
Options:
  --show-url             also print the chosen signed URL (secret: anyone holding it can download)
  --user-data <dir>      app data folder holding the Chrome profile (default: the desktop app's)
  --credentials <file>   account file (default: ${DEFAULT_CREDENTIALS})
  --headful              show the Chrome window
Exit codes: 0 done, 1 error, 3 not logged in.`;

function parseArgs(argv) {
  const opts = { userData: defaultUserDataDir(), credentialsFile: DEFAULT_CREDENTIALS, headless: true, showUrl: false };
  const value = (i, flag) => {
    if (argv[i + 1] === undefined || argv[i + 1].startsWith('--')) throw new Error(`${flag} needs a value`);
    return argv[i + 1];
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case '--help': case '-h': opts.help = true; break;
      case '--links': opts.links = value(i, a); i++; break;
      case '--public': opts.public = value(i, a); i++; break;
      case '--show-url': opts.showUrl = true; break;
      case '--user-data': opts.userData = path.resolve(value(i, a)); i++; break;
      case '--credentials': opts.credentialsFile = path.resolve(value(i, a)); i++; break;
      case '--headful': opts.headless = false; break;
      default: throw new Error(`Unknown option: ${a}`);
    }
  }
  return opts;
}

const expiryText = (epoch) => (epoch ? `${new Date(epoch * 1000).toISOString()} (in ${Math.round((epoch * 1000 - Date.now()) / 3600_000)} h)` : 'unknown');

async function main() {
  let opts;
  try { opts = parseArgs(process.argv.slice(2)); } catch (err) { console.error(`${err.message}\n\n${USAGE}`); return 1; }
  if (opts.help || (!opts.links && !opts.public)) { console.log(USAGE); return opts.help ? 0 : 1; }

  if (opts.public) {
    const facts = await scenePublic(opts.public);
    if (!facts) { console.error(`No VRBangers scene "${opts.public}"`); return 1; }
    console.log(JSON.stringify(facts, null, 2));
    return 0;
  }

  readCredentials(opts.credentialsFile);
  const vrb = await openVrbangers({ ...opts, log: (m) => console.error(`  ${m}`) });
  try {
    await vrb.ensureLoggedIn();
    const links = await vrb.sceneLinks(opts.links);
    if (!links.length) { console.error(`No downloads found for "${opts.links}" (wrong slug, or the account cannot download it)`); return 1; }
    console.log(`Downloads for ${opts.links}:`);
    for (const l of links) console.log(`  ${l.quality.padStart(3)}  ${l.file}`);
    const best = lib.chooseLink(links);
    if (!best) { console.error(`None within the VR caps (≤ ${lib.VR_MAX_K}K)`); return 1; }
    console.log(`Chosen: ${best.file}  quality ${best.quality} (nominal width ${best.width})  projection ${best.projection}  expires ${expiryText(best.expires)}`);
    console.log(`URL: ${opts.showUrl ? best.url : lib.redactUrl(best.url)}`);
    return 0;
  } finally {
    await vrb.close();
  }
}

if (IS_CLI) {
  main().then((code) => process.exit(code), (err) => {
    console.error(err instanceof LoginError ? err.message : (err.stack || err.message));
    process.exit(err.exitCode || 1);
  });
}
