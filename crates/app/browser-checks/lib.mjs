// Shared settings for the browser checks. Everything points at a local stack
// with fake data only (see README.md). Override with environment variables:
//   BASE       site URL               (default http://127.0.0.1:3031)
//   STATE_DIR  seed output, sessions  (default ./.state)
//   REPORT_DIR JSON reports           (default ./reports)
//   HEADED=1   show the browser
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const B = (process.env.BASE || 'http://127.0.0.1:3031').replace(/\/$/, '');
export const STATE_DIR = path.resolve(process.env.STATE_DIR || path.join(HERE, '.state'));
export const REPORT_DIR = path.resolve(process.env.REPORT_DIR || path.join(HERE, 'reports'));
fs.mkdirSync(STATE_DIR, { recursive: true });
fs.mkdirSync(REPORT_DIR, { recursive: true });

// Fake accounts created by seed.mjs. The password is test data, not a secret.
export const PW = 'Test-password-12345';
export const USERS = { member: 'Player01', admin: 'TestAdmin', officer: 'TestOfficer', applicant: 'Applicant01', recruit: 'Recruit01' };
export const VIEW = { 1440: { width: 1440, height: 900 }, 390: { width: 390, height: 844 }, 360: { width: 360, height: 740 } };

export function readState(name) { return JSON.parse(fs.readFileSync(path.join(STATE_DIR, name), 'utf8')); }
export function writeState(name, value) { fs.writeFileSync(path.join(STATE_DIR, name), JSON.stringify(value, null, 1)); }

export async function launch() { return chromium.launch({ headless: process.env.HEADED !== '1' }); }
export async function ready(page) {
  await page.waitForSelector('.d1-top', { timeout: 30000 });
  await page.waitForFunction(() => !document.querySelector('.d1-session-pending'), null, { timeout: 15000 });
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(900);
}
export async function open(page, p) { await page.goto(B + p, { waitUntil: 'load' }); await ready(page); }

// Visible loading markers: skeleton lists, spinners, aria-busy, and any
// element whose class names a loading state ("admin-loading", "loading-state",
// "wiki-loading"...). An error that reuses such a class is not loading.
const LOADING = '.ui-skel-list, .ui-spinner, [aria-busy="true"], [class*="loading"]';
/** Wait until no loading marker is visible. Returns the ones left after `ms` (empty when settled). */
export async function settled(page, ms = 10000) {
  const left = () => page.evaluate(sel => [...document.querySelectorAll(sel)]
    .filter(e => e.getBoundingClientRect().height > 0 && !/failed|error/i.test(e.textContent))
    .map(e => e.className + ': ' + (e.textContent || '').trim().slice(0, 40)), LOADING);
  try {
    await page.waitForFunction(sel => ![...document.querySelectorAll(sel)]
      .some(e => e.getBoundingClientRect().height > 0 && !/failed|error/i.test(e.textContent)), LOADING, { timeout: ms });
    return [];
  } catch { return left(); }
}

/** A browser context signed in as `role` (or a guest). Sessions are cached in STATE_DIR. */
export async function makeContext(browser, role, width, theme = 'dark') {
  const statePath = path.join(STATE_DIR, 'session-' + role + '.json');
  const opts = { viewport: VIEW[width], deviceScaleFactor: 2, colorScheme: theme, hasTouch: width < 820 };
  if (role !== 'guest' && fs.existsSync(statePath)) opts.storageState = statePath;
  const ctx = await browser.newContext(opts);
  await ctx.addInitScript(t => { try { localStorage.setItem('sc-theme', t); } catch {} }, theme);
  if (role !== 'guest' && !fs.existsSync(statePath)) {
    const p = await ctx.newPage();
    await p.goto(B + '/', { waitUntil: 'load' });
    const st = await p.evaluate(async ([u, pw]) => (await fetch('/api/auth/local/login', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ username: u, password: pw }) })).status, [USERS[role], PW]);
    if (st !== 200) throw new Error('login ' + role + ' ' + st);
    await ctx.storageState({ path: statePath });
    await p.close();
  }
  return ctx;
}

/** Collects PASS/FAIL lines and writes reports/<name>.json. Exit code 1 on any failure. */
export function reporter(name) {
  const res = [];
  const rec = (title, ok, detail = '') => { res.push({ name: title, ok: !!ok, detail }); console.log((ok ? 'PASS ' : 'FAIL ') + title + (detail && !ok ? '  [' + detail + ']' : '')); };
  const finish = () => {
    const pass = res.filter(r => r.ok).length;
    fs.writeFileSync(path.join(REPORT_DIR, name + '.json'), JSON.stringify(res, null, 1));
    console.log(`\n${name}: ${pass} pass, ${res.length - pass} fail`);
    process.exit(pass === res.length && res.length > 0 ? 0 : 1);
  };
  return { rec, finish, res };
}

/** Cookie-jar HTTP client for the seed scripts. */
export class Jar {
  constructor() { this.c = {}; }
  async req(method, p, body) {
    const h = { Origin: B, 'Content-Type': 'application/json', Cookie: Object.entries(this.c).map(([k, v]) => `${k}=${v}`).join('; ') };
    const r = await fetch(B + p, { method, headers: h, body: body === undefined ? undefined : JSON.stringify(body) });
    for (const sc of r.headers.getSetCookie()) { const [kv] = sc.split(';'); const i = kv.indexOf('='); this.c[kv.slice(0, i)] = kv.slice(i + 1); }
    const t = await r.text(); let j; try { j = JSON.parse(t); } catch { j = t; }
    console.log(method, p, r.status, typeof j === 'string' ? j.slice(0, 160) : JSON.stringify(j).slice(0, 200));
    return { status: r.status, j };
  }
  async login(user) { const r = await this.req('POST', '/api/auth/local/login', { username: user, password: PW }); if (r.status !== 200) throw new Error('login ' + user + ' ' + r.status); return this; }
}
export const sleep = ms => new Promise(r => setTimeout(r, ms));
