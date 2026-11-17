// Shared settings for the S4a browser checks. Everything points at a local
// stack with fake data only (see README.md). Override with environment
// variables:
//   BASE       site URL                (default http://127.0.0.1:3031)
//   STATE_DIR  seed output, sessions   (default ./.state)
//   REPORT_DIR JSON reports            (default ./reports)
//   SHOTS_DIR  screenshots             (default ./shots)
//   HEADED=1   show the browser
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const B = (process.env.BASE || 'http://127.0.0.1:3031').replace(/\/$/, '');
export const STATE_DIR = path.resolve(process.env.STATE_DIR || path.join(HERE, '.state'));
export const REPORT_DIR = path.resolve(process.env.REPORT_DIR || path.join(HERE, 'reports'));
export const SHOTS = path.resolve(process.env.SHOTS_DIR || path.join(HERE, 'shots'));
fs.mkdirSync(STATE_DIR, { recursive: true });
fs.mkdirSync(REPORT_DIR, { recursive: true });
// Fake accounts created by seed.mjs. The password is test data, not a secret.
export const PW = 'Test-password-12345';
// member = Player01 (active member), visitor = Visitor01 (account, no application: the
// only viewer who gets the Apply form), applicant = Applicant01 (pending application).
export const USERS = { member: 'Player01', admin: 'TestAdmin', visitor: 'Visitor01', applicant: 'Applicant01' };
export const VIEW = { 1440: { width: 1440, height: 900 }, 390: { width: 390, height: 844 } };
export const sleep = ms => new Promise(r => setTimeout(r, ms));
export const EVENT = { title: 'Scrim night', day_of_week: 2, time: '19:30', timezone: 'Europe/Stockholm', duration_minutes: 120, is_recurring: true, is_public: true };

// Cookie jar over fetch, for seeding through the app's own API.
export class Jar {
  constructor(quiet = false) { this.c = {}; this.quiet = quiet; }
  async req(method, path, body, raw) {
    const h = { Origin: B, Cookie: Object.entries(this.c).map(([k, v]) => `${k}=${v}`).join('; ') };
    if (!raw) h['Content-Type'] = 'application/json';
    const r = await fetch(B + path, { method, headers: h, body: raw ?? (body === undefined ? undefined : JSON.stringify(body)) });
    for (const sc of r.headers.getSetCookie()) { const [kv] = sc.split(';'); const i = kv.indexOf('='); this.c[kv.slice(0, i)] = kv.slice(i + 1); }
    const t = await r.text(); let j; try { j = JSON.parse(t); } catch { j = t; }
    if (!this.quiet) console.log(method, path, r.status, (typeof j === 'string' ? j : JSON.stringify(j)).slice(0, 160));
    return { status: r.status, j };
  }
  async login(name) { const r = await this.req('POST', '/api/auth/local/login', { username: name, password: PW }); if (r.status !== 200) throw new Error('login ' + name + ' ' + r.status); return this; }
}
export const rows = j => Array.isArray(j) ? j : (j && (j.data || j.items)) || [];
export async function adminJar(quiet = true) { return new Jar(quiet).login('TestAdmin'); }
export async function deleteEvents(admin) { for (const e of rows((await admin.req('GET', '/api/events')).j)) await admin.req('DELETE', '/api/events/' + e.id); }
export async function ensureEvent(admin) {
  const have = rows((await admin.req('GET', '/api/events')).j);
  if (!have.some(e => e.title === EVENT.title)) await admin.req('POST', '/api/events', EVENT);
}

export async function launch() { return chromium.launch({ headless: process.env.HEADED !== '1' }); }
export async function ready(page) {
  await page.waitForSelector('.d1-top', { timeout: 30000 });
  await page.waitForFunction(() => !document.querySelector('.d1-session-pending'), null, { timeout: 15000 });
  // Data loaded: no skeleton bars or rows left in the page.
  await page.waitForFunction(() => !document.querySelector('.ui-skel-list, .ui-skel-row, .home-skel, [aria-busy="true"]'), null, { timeout: 15000 }).catch(() => {});
  await page.evaluate(() => document.fonts.ready);
  await page.waitForLoadState('networkidle', { timeout: 8000 }).catch(() => {});
  await page.waitForTimeout(500);
}
export async function open(page, path) { await page.goto(B + path, { waitUntil: 'load' }); await ready(page); }
export async function makeContext(browser, role, width, theme) {
  const statePath = path.join(STATE_DIR, 'session-' + role + '.json');
  const opts = { viewport: VIEW[width], deviceScaleFactor: 2, colorScheme: theme, hasTouch: width === 390 };
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
