// Screenshots plus the in-page audit for every capture.
//   node shots.mjs            all roles, widths and themes
//   node shots.mjs guest-390  only keys containing the filter (role-width-theme)
// Shots go to $SHOTS_DIR, audit results to $REPORT_DIR/report-audit.json
// (merged on partial runs). Run summary.mjs for the grouped failure list.
import fs from 'node:fs';
import { launch, makeContext, open, ready, SHOTS, REPORT_DIR, adminJar, deleteEvents, ensureEvent } from './lib.mjs';
import { audit } from './audit.mjs';

const only = process.argv[2];
const REPORT = REPORT_DIR + '/report-audit.json';
const POST = '/blog/scrim-review-three-weeks';
const FULL = [
  ['home', '/'], ['blog', '/blog'], ['blog-post', POST], ['news', '/news'], ['wiki', '/wiki'], ['wiki-page', '/wiki/tank-basics'],
  ['events', '/events'], ['apply', '/apply'], ['404', '/no-such-page'], ['no-access', '/admin'],
  ['blog-post-404', '/blog/no-such-post'], ['wiki-404', '/wiki/no-such-topic'],
];
const ROLES = { guest: FULL, member: FULL, visitor: [['apply', '/apply']], applicant: [['apply', '/apply']] };
const report = fs.existsSync(REPORT) && only ? JSON.parse(fs.readFileSync(REPORT, 'utf8')) : {};
fs.mkdirSync(SHOTS, { recursive: true });
const combos = [];
for (const role of Object.keys(ROLES)) for (const width of [1440, 390]) for (const theme of ['dark', 'light']) {
  const key = `${role}-${width}-${theme}`; if (only && !key.includes(only)) continue; combos.push({ role, width, theme, key });
}

const browser = await launch();
async function session(c) {
  const ctx = await makeContext(browser, c.role, c.width, c.theme);
  const page = await ctx.newPage(); const errs = []; const bad = [];
  page.on('console', m => { if (m.type() === 'error') errs.push(m.text().slice(0, 240)); });
  page.on('pageerror', e => errs.push('PAGEERROR ' + String(e).slice(0, 240)));
  page.on('response', r => { if (r.status() >= 400) bad.push(`${r.status()} ${r.request().method()} ${new URL(r.url()).pathname}`); });
  const capture = async name => {
    // Park the pointer so no card is caught in its hover state.
    await page.mouse.move(0, 0);
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.waitForTimeout(150);
    const a = await page.evaluate(audit);
    await page.screenshot({ path: `${SHOTS}/${c.role}-${name}-${c.width}-${c.theme}.png`, fullPage: true });
    report[`${c.role}-${name}-${c.width}-${c.theme}`] = { role: c.role, page: name, w: c.width, theme: c.theme, ...a, errs: [...errs], http: [...bad] };
    errs.length = 0; bad.length = 0;
  };
  const go = async path => { errs.length = 0; bad.length = 0; await open(page, path); };
  return { ctx, page, capture, go };
}

for (const c of combos) {
  const { ctx, page, capture, go } = await session(c);
  for (const [name, path] of ROLES[c.role]) {
    await go(path);
    await capture(name);
    if (name === 'blog') {
      // Pager on page 2.
      await page.click('#blog-pager-next');
      await page.waitForFunction(() => { const b = document.querySelector('#blog-pager [aria-current="page"]'); return b && b.textContent.trim() === '2'; }, null, { timeout: 5000 }).catch(() => {});
      await page.waitForTimeout(400);
      await capture('blog-page2');
    }
    if (name === 'apply' && c.role === 'visitor') {
      // Submit the empty form: error summary plus inline errors.
      await page.click('.apply-form button[type=submit]');
      await page.waitForSelector('#apply-error-summary', { timeout: 5000 }).catch(() => {});
      await page.waitForTimeout(300);
      await capture('apply-errors');
    }
    if (name === 'apply' && c.role === 'applicant') {
      // Withdraw confirm strip open.
      await page.click('#withdraw-application-trigger');
      await page.waitForSelector('.inline-confirm', { timeout: 5000 }).catch(() => {});
      await page.waitForTimeout(300);
      await capture('apply-withdraw');
    }
    if (name === 'wiki-page') {
      // History panel open (it sits under the article).
      await page.click('button[aria-controls$="-history-panel"]');
      await page.waitForSelector('.wiki-revisions:not(.is-closed)', { timeout: 5000 }).catch(() => {});
      await page.waitForTimeout(600);
      await capture('wiki-history');
    }
    if (name === 'wiki' && c.role === 'guest') {
      // Search with no hits.
      await page.fill('#wiki-search', 'zzzz no such page');
      await page.waitForSelector('.ui-empty, .empty-card, section#wiki-list h2', { timeout: 5000 }).catch(() => {});
      await page.waitForTimeout(900);
      await capture('wiki-nomatch');
    }
    if (name === 'blog' && c.role === 'guest') {
      // Failed list load: the API is stubbed to answer 500.
      await page.route('**/api/articles?*', r => r.fulfill({ status: 500, contentType: 'application/json', body: '{"error":"stub"}' }));
      await go('/blog');
      await capture('blog-failed');
      await page.unroute('**/api/articles?*');
    }
  }
  if (ROLES[c.role] === FULL) {
    await go('/patch-notes');
    await page.waitForSelector('.pn-row-head', { timeout: 10000 }).catch(() => {});
    const first = page.locator('.pn-row-head').first();
    if ((await first.getAttribute('aria-expanded')) !== 'true') { await first.click(); await page.waitForTimeout(300); }
    await capture('patch-notes');
    await page.click('.pn-expand'); await page.waitForTimeout(400);
    await capture('patch-notes-expanded');
  }
  await ctx.close();
  console.log('done', c.key);
}

// Empty events state: remove the event, shoot, put it back.
const emptyCombos = combos.filter(c => ROLES[c.role] === FULL);
if (emptyCombos.length) {
  const admin = await adminJar();
  await deleteEvents(admin);
  try {
    for (const c of emptyCombos) {
      const { ctx, capture, go } = await session(c);
      await go('/events'); await capture('events-empty');
      await ctx.close();
    }
  } finally { await ensureEvent(admin); }
  console.log('done events-empty, event restored');
  // Left aligned hero (the default alignment): switch the setting, shoot, restore.
  const cur = (await admin.req('GET', '/api/settings')).j; const hp = cur.homepage || {};
  await admin.req('PUT', '/api/settings', { homepage: { ...hp, content_align: 'left' } });
  try {
    for (const c of emptyCombos.filter(c => c.role === 'guest')) {
      const { ctx, capture, go } = await session(c);
      await go('/'); await capture('home-left');
      await ctx.close();
    }
  } finally { await admin.req('PUT', '/api/settings', { homepage: { ...hp, content_align: hp.content_align || 'center' } }); }
  console.log('done home-left, alignment restored');
}
await browser.close();
fs.writeFileSync(REPORT, JSON.stringify(report, null, 1));
console.log('captures:', Object.keys(report).length, '->', REPORT);
await import('./summary.mjs');
