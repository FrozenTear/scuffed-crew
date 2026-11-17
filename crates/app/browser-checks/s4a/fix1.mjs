// Browser check for the S4a review fixes (PR 696, FIX1).
//   node fix1.mjs            every group
//   node fix1.mjs apply      only groups whose name contains the filter
// Shots go to $SHOTS_DIR/fix1, results to $REPORT_DIR/report-fix1.json.
// Covers apply (the decided rules: main role plus a 10 character message),
// the pager and patch notes in normal and forced colours at 1440 and 390,
// plus the other fixes that need a browser to see.
import fs from 'node:fs';
import { B, PW, SHOTS as SHOTS_ROOT, REPORT_DIR, VIEW, Jar, rows, sleep, launch, open, ready, adminJar, EVENT } from './lib.mjs';
import { audit } from './audit.mjs';

const SHOTS = SHOTS_ROOT + '/fix1';
fs.mkdirSync(SHOTS, { recursive: true });
const only = process.argv[2];
const want = g => !only || g.includes(only);
const results = [];
const check = (group, name, pass, detail = '') => {
  results.push({ group, name, pass: !!pass, detail: String(detail).slice(0, 400) });
  console.log((pass ? 'PASS ' : 'FAIL ') + group + ': ' + name + (detail ? '  [' + String(detail).slice(0, 260) + ']' : ''));
};
const audits = {};
const browser = await launch();
const admin = await adminJar(true);

async function ctxFor(width, forced, user) {
  const ctx = await browser.newContext({
    viewport: VIEW[width], deviceScaleFactor: 2, colorScheme: 'dark', hasTouch: width === 390,
    forcedColors: forced ? 'active' : 'none',
  });
  await ctx.addInitScript(() => { try { localStorage.setItem('sc-theme', 'dark'); } catch {} });
  const page = await ctx.newPage();
  const errs = [];
  page.on('pageerror', e => errs.push('PAGEERROR ' + String(e).slice(0, 200)));
  if (user) {
    await page.goto(B + '/', { waitUntil: 'load' });
    const st = await page.evaluate(async ([u, pw]) => (await fetch('/api/auth/local/login', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ username: u, password: pw }) })).status, [user, PW]);
    if (st !== 200) throw new Error('login ' + user + ' ' + st);
  }
  return { ctx, page, errs };
}
const mode = forced => (forced ? 'forced' : 'normal');
async function shot(page, name, width, forced, sel) {
  await page.mouse.move(0, 0);
  const path = `${SHOTS}/${name}-${width}-${mode(forced)}.png`;
  // Playwright's stitched full-page capture does not paint forced colours
  // (it comes out in the author colours), so forced shots are the viewport.
  if (sel) await page.locator(sel).first().screenshot({ path });
  else await page.screenshot({ path, fullPage: !forced });
  const a = await page.evaluate(audit);
  audits[`${name}-${width}-${mode(forced)}`] = { overflowX: a.overflowX, unnamed: a.unnamed, small: a.small, targets: a.targets, dupIds: a.dupIds, h1: a.h1, mains: a.mains, contrast: forced ? 'not measured in forced colours' : a.contrast.filter(c => !c.disabled) };
  // An aria-disabled control is exempt from 4.5:1 (the pager's dead step is 3.71).
  return a;
}
async function register(name) {
  const j = new Jar(true);
  let r = await j.req('POST', '/api/auth/local/login', { username: name, password: PW });
  if (r.status !== 200) {
    await sleep(2500);
    r = await j.req('POST', '/api/auth/local/register', { username: name, password: PW, confirm_min_age: true });
    if (r.status >= 300) throw new Error('register ' + name + ' ' + r.status + ' ' + JSON.stringify(r.j));
  }
  return j;
}
const toastText = page => page.evaluate(() => [...document.querySelectorAll('.toast, [class*="toast"]')].map(t => t.textContent.trim()).filter(Boolean).join(' | '));
async function waitToast(page, re, ms = 6000) {
  const end = Date.now() + ms;
  while (Date.now() < end) { const t = await toastText(page); if (re.test(t)) return t; await page.waitForTimeout(150); }
  return await toastText(page);
}
const games = rows((await admin.req('GET', '/api/games')).j);
const ow = games.find(g => g.name === 'Overwatch');

// ---------------------------------------------------------------------------
// Apply: the decided rules, in normal and forced colours, 1440 and 390.
// ---------------------------------------------------------------------------
if (want('apply')) {
  let n = 0;
  for (const width of [1440, 390]) for (const forced of [false, true]) {
    const tag = ` @${width} ${mode(forced)}`;
    const user = `Visitor1${n++}`;
    await register(user);
    const { ctx, page } = await ctxFor(width, forced, user);
    await open(page, '/apply');
    check('apply', 'form shows for an account with no application' + tag, await page.locator('.apply-form').count() === 1);
    const hint = await page.locator('#apply-message-hint').textContent();
    check('apply', 'hint says 10 characters before the first submit' + tag, hint.trim() === 'At least 10 characters.', hint);
    const req = await page.evaluate(() => ['apply-role', 'apply-message'].map(id => document.getElementById(id).required));
    check('apply', 'role and message are required controls' + tag, req.every(Boolean), JSON.stringify(req));
    // Empty submit: three errors, focus on the summary, no live region.
    await page.click('.apply-form button[type=submit]');
    await page.waitForSelector('#apply-error-summary');
    await page.waitForTimeout(300);
    const sum = await page.evaluate(() => { const s = document.getElementById('apply-error-summary'); return { focused: document.activeElement === s, role: s.getAttribute('role'), live: s.getAttribute('aria-live'), name: (document.getElementById(s.getAttribute('aria-labelledby')) || {}).textContent, links: [...s.querySelectorAll('a')].map(a => a.textContent.trim()) }; });
    check('apply', 'empty submit lists games, role and message' + tag, sum.links.length === 3, JSON.stringify(sum.links));
    check('apply', 'message error says 10 characters, not 40' + tag, /at least 10 characters/.test(sum.links[2] || '') && !/40/.test(sum.links.join(' ')), sum.links[2]);
    check('apply', 'summary takes focus and is not an alert' + tag, sum.focused && sum.role === 'group' && !sum.live && /could not be sent/.test(sum.name || ''), JSON.stringify(sum));
    check('apply', 'no role=alert in the form' + tag, await page.locator('.apply-form [role=alert]').count() === 0);
    await shot(page, 'apply-errors', width, forced);
    // Fill it in: a game, a role, and a 9 character message is still refused.
    await page.locator('.apply-form .chip', { hasText: 'Overwatch' }).first().click();
    await page.selectOption('#apply-role', 'Support');
    await page.fill('#apply-message', '123456789');
    await page.waitForTimeout(200);
    const left9 = await page.evaluate(() => [...document.querySelectorAll('#apply-error-summary a')].map(a => a.getAttribute('href')));
    check('apply', 'with a game and role chosen only the message error is left' + tag, left9.length === 1 && left9[0] === '#apply-message', JSON.stringify(left9));
    let posted = 0; page.on('request', r => { if (r.method() === 'POST' && r.url().endsWith('/api/applications')) posted++; });
    await page.click('.apply-form button[type=submit]'); await page.waitForTimeout(500);
    check('apply', '9 characters is refused in the browser, nothing is sent' + tag, posted === 0 && await page.locator('#apply-message[aria-invalid=true]').count() === 1, 'posts=' + posted);
    // No role: refused too.
    await page.fill('#apply-message', 'Hi, I heal');
    await page.selectOption('#apply-role', '');
    await page.click('.apply-form button[type=submit]'); await page.waitForTimeout(400);
    const noRole = await page.evaluate(() => [...document.querySelectorAll('#apply-error-summary a')].map(a => a.getAttribute('href')));
    check('apply', 'no main role is refused, nothing is sent' + tag, posted === 0 && noRole.length === 1 && noRole[0] === '#apply-role', JSON.stringify(noRole));
    await page.selectOption('#apply-role', 'Support');
    await shot(page, 'apply-filled', width, forced);
    if (n % 2 === 1) {
      // Exactly 10 characters is accepted, and the payload is the old one
      // plus the role as the one preferred role.
      const [reqSent] = await Promise.all([
        page.waitForRequest(r => r.method() === 'POST' && r.url().endsWith('/api/applications')),
        page.click('.apply-form button[type=submit]'),
      ]);
      const body = reqSent.postDataJSON();
      check('apply', 'payload: preferred_games, preferred_roles [role], message' + tag, JSON.stringify(Object.keys(body).sort()) === '["message","preferred_games","preferred_roles"]' && JSON.stringify(body.preferred_roles) === '["Support"]' && body.message === 'Hi, I heal' && body.preferred_games.length === 1 && body.preferred_games[0] === ow.id, JSON.stringify(body));
      const resp = await reqSent.response();
      check('apply', 'the API accepts it (201 or 200)' + tag, resp && resp.status() < 300, resp && resp.status());
      await page.waitForSelector('#apply-status-heading', { timeout: 8000 }).catch(() => {});
      await page.waitForTimeout(500);
      const head = await page.evaluate(() => { const h = document.getElementById('apply-status-heading'); return h ? { text: h.textContent.trim(), focused: document.activeElement === h } : null; });
      check('apply', '10 characters is accepted: "Application received" takes focus' + tag, head && head.text === 'Application received' && head.focused, JSON.stringify(head));
      await shot(page, 'apply-received', width, forced);
    } else {
      // The server refuses: 409 because an application now exists (sent
      // through the API behind the open form). The toast gives its reason.
      const j = new Jar(true); await j.login(user);
      const made = await j.req('POST', '/api/applications', { preferred_games: [ow.id], preferred_roles: ['Support'], message: 'Sent from another tab.' });
      await page.click('.apply-form button[type=submit]');
      const t = await waitToast(page, /Couldn't send/);
      check('apply', '409 shows the server reason, not "try again"' + tag, made.status < 300 && /Couldn't send the application: You already have an open application \(pending\)\./.test(t) && !/try again/i.test(t), t);
      await page.screenshot({ path: `${SHOTS}/apply-409-${width}-${mode(forced)}.png` });
      await j.req('POST', '/api/applications/mine/withdraw', {});
    }
    await ctx.close();
  }
  // 403: recruitment closed behind an open form.
  {
    const user = 'Visitor19'; await register(user);
    const { ctx, page } = await ctxFor(1440, false, user);
    await open(page, '/apply');
    await page.locator('.apply-form .chip', { hasText: 'Overwatch' }).first().click();
    await page.selectOption('#apply-role', 'Tank');
    await page.fill('#apply-message', 'Tank main, evenings.');
    const settings = (await admin.req('GET', '/api/settings')).j;
    const close = await admin.req('PUT', '/api/settings', { ...settings, recruitment_open: false });
    let closed = close.status < 300;
    if (!closed) { const p = await admin.req('PATCH', '/api/settings', { recruitment_open: false }); closed = p.status < 300; }
    if (closed) {
      await page.click('.apply-form button[type=submit]');
      const t = await waitToast(page, /Couldn't send/);
      check('apply', '403 (recruitment closed) shows the server reason, not "try again"', /Couldn't send the application: recruitment is currently closed\./.test(t) && !/try again/i.test(t), t);
      await page.screenshot({ path: `${SHOTS}/apply-403-1440-normal.png` });
      const back = await admin.req('PUT', '/api/settings', { ...settings, recruitment_open: true });
      if (back.status >= 300) await admin.req('PATCH', '/api/settings', { recruitment_open: true });
      const now = (await admin.req('GET', '/api/settings')).j;
      check('apply', 'recruitment reopened after the 403 check', now.recruitment_open === true);
    } else check('apply', 'could close recruitment for the 403 check', false, JSON.stringify(close.j).slice(0, 200));
    await ctx.close();
  }
}

// ---------------------------------------------------------------------------
// Pager (Blog, two pages) in normal and forced colours.
// ---------------------------------------------------------------------------
if (want('pager')) {
  for (const width of [1440, 390]) for (const forced of [false, true]) {
    const tag = ` @${width} ${mode(forced)}`;
    const { ctx, page } = await ctxFor(width, forced, null);
    await open(page, '/blog');
    await page.waitForSelector('#blog-pager [aria-current="page"]');
    const st = await page.evaluate(() => {
      const cs = el => { const s = getComputedStyle(el); return { bw: s.borderTopWidth, bs: s.borderTopStyle, bc: s.borderTopColor, color: s.color, bg: s.backgroundColor, weight: s.fontWeight, deco: s.textDecorationLine, shadow: s.boxShadow, h: Math.round(el.getBoundingClientRect().height), w: Math.round(el.getBoundingClientRect().width) }; };
      const probe = name => { const d = document.createElement('div'); d.style.color = name; d.style.forcedColorAdjust = 'none'; document.body.appendChild(d); const c = getComputedStyle(d).color; d.remove(); return c; };
      const cur = document.querySelector('#blog-pager [aria-current="page"]');
      const other = [...document.querySelectorAll('#blog-pager .page-btn')].find(b => !b.hasAttribute('aria-current'));
      const off = document.querySelector('#blog-pager .is-disabled');
      return { cur: cs(cur), other: cs(other), off: cs(off), offText: off.textContent.trim(), sys: { Highlight: probe('Highlight'), GrayText: probe('GrayText'), ButtonText: probe('ButtonText') }, forced: matchMedia('(forced-colors: active)').matches };
    });
    check('pager', 'media query matches the mode' + tag, st.forced === forced);
    check('pager', 'current page is heavier than the other pages (700 vs 600)' + tag, Number(st.cur.weight) >= Number(st.other.weight) + 100, `${st.cur.weight} vs ${st.other.weight}`);
    check('pager', 'current and other page buttons are the same size (44px high)' + tag, st.cur.h === st.other.h && st.cur.h >= 44 && st.cur.w === st.other.w, `${st.cur.w}x${st.cur.h} vs ${st.other.w}x${st.other.h}`);
    if (forced) {
      check('pager', 'forced: current page has a 3px Highlight edge and an underline' + tag, st.cur.bw === '3px' && st.cur.bs === 'solid' && st.cur.bc === st.sys.Highlight && /underline/.test(st.cur.deco), JSON.stringify(st.cur));
      check('pager', 'forced: other pages keep a 1px edge and no underline' + tag, st.other.bw === '1px' && !/underline/.test(st.other.deco), JSON.stringify(st.other));
      check('pager', 'forced: disabled ' + st.offText + ' is GrayText with a dashed edge' + tag, st.off.color === st.sys.GrayText && st.off.bs === 'dashed' && st.off.bc === st.sys.GrayText, JSON.stringify(st.off));
    } else {
      check('pager', 'normal: current page keeps fill plus doubled edge' + tag, st.cur.bg !== st.other.bg && st.cur.shadow !== 'none' && st.cur.bw === '1px', JSON.stringify(st.cur));
      check('pager', 'normal: disabled step is not dashed' + tag, st.off.bs === 'solid', JSON.stringify(st.off));
    }
    await page.locator('#blog-pager').scrollIntoViewIfNeeded();
    await shot(page, 'pager-blog-p1', width, forced, '#blog-pager');
    await page.waitForTimeout(200);
    await shot(page, 'blog-p1', width, forced);
    await page.click('#blog-pager-next');
    await page.waitForFunction(() => { const b = document.querySelector('#blog-pager [aria-current="page"]'); return b && b.textContent.trim() === '2'; });
    await page.waitForTimeout(400);
    const p2 = await page.evaluate(() => ({ url: location.search, range: document.querySelector('#blog-pager-meta').textContent.trim(), nextOff: document.querySelector('#blog-pager-next').getAttribute('aria-disabled'), cards: document.querySelectorAll('.entry-row').length }));
    check('pager', 'page 2: address, range line, Next disabled, 4 cards' + tag, p2.url === '?page=2' && /11 to 14 of 14/.test(p2.range) && p2.nextOff === 'true' && p2.cards === 4, JSON.stringify(p2));
    await page.locator('#blog-pager').scrollIntoViewIfNeeded();
    await shot(page, 'pager-blog-p2', width, forced, '#blog-pager');
    await ctx.close();
  }
}

// ---------------------------------------------------------------------------
// Patch notes in normal and forced colours.
// ---------------------------------------------------------------------------
if (want('patch')) {
  for (const width of [1440, 390]) for (const forced of [false, true]) {
    const tag = ` @${width} ${mode(forced)}`;
    const { ctx, page } = await ctxFor(width, forced, null);
    await open(page, '/patch-notes');
    await page.waitForSelector('.pn-row');
    const head = await page.evaluate(() => {
      const input = document.getElementById('patch-notes-search'); const label = document.querySelector('label[for="patch-notes-search"]');
      const lr = label && label.getBoundingClientRect();
      const levels = [...document.querySelectorAll('main h1, main h2, main h3, main h4')].map(h => h.tagName + (h.className ? '.' + h.className.split(' ')[0] : ''));
      return { label: label && label.textContent.trim(), labelVisible: !!lr && lr.width > 10 && lr.height > 10, aria: input.getAttribute('aria-label'), placeholder: input.placeholder, inputW: Math.round(input.getBoundingClientRect().width), sameRow: !!lr && Math.abs((lr.top + lr.bottom) / 2 - (input.getBoundingClientRect().top + input.getBoundingClientRect().bottom) / 2) < 6, years: [...document.querySelectorAll('h2.pn-year')].map(h => h.textContent.trim()), rowTags: [...new Set([...document.querySelectorAll('.pn-row-title')].map(h => h.tagName))], levels: levels.slice(0, 8) };
    });
    check('patch notes', 'search has a visible label on the row, no aria-label' + tag, head.label === 'Search' && head.labelVisible && head.sameRow && head.aria === null, JSON.stringify(head));
    check('patch notes', 'search field keeps room for its hint' + tag, head.inputW >= 150, head.inputW + 'px, placeholder ' + head.placeholder);
    check('patch notes', 'years are h2 headings and patch rows h3' + tag, head.years.length >= 1 && head.rowTags.join() === 'H3', JSON.stringify({ years: head.years, rowTags: head.rowTags, levels: head.levels }));
    // Open every row, then every hero of the first patch that has heroes, so
    // the Buff / Nerf / Fix / Adjust marks are on screen.
    const expand = page.locator('.pn-expand');
    if (await expand.count()) { await expand.click(); await page.waitForTimeout(400); }
    const heroHeads = page.locator('.pn-hero-head');
    const heroCount = await heroHeads.count();
    for (let i = 0; i < Math.min(heroCount, 6); i++) { const h = heroHeads.nth(i); if (await h.getAttribute('aria-expanded') === 'false') { await h.click(); await page.waitForTimeout(150); } }
    await page.waitForTimeout(300);
    const marks = await page.evaluate(() => {
      const probe = name => { const d = document.createElement('div'); d.style.color = name; d.style.forcedColorAdjust = 'none'; document.body.appendChild(d); const c = getComputedStyle(d).color; d.remove(); return c; };
      const canvasText = probe('CanvasText'); const canvas = probe('Canvas');
      const out = { canvasText, canvas, kinds: {} };
      for (const kind of ['up', 'down', 'diamond', 'dot']) {
        const els = [...document.querySelectorAll('.pn-mark-' + kind)].filter(e => e.getBoundingClientRect().width > 0 || e.getBoundingClientRect().height > 0);
        if (!els.length) { out.kinds[kind] = null; continue; }
        const s = getComputedStyle(els[0]); const r = els[0].getBoundingClientRect();
        out.kinds[kind] = { n: els.length, adjust: s.forcedColorAdjust, color: s.color, bg: s.backgroundColor, top: s.borderTopColor, bottom: s.borderBottomColor, left: s.borderLeftColor, w: Math.round(r.width * 10) / 10, h: Math.round(r.height * 10) / 10, word: els[0].parentElement.textContent.trim() };
      }
      return out;
    });
    const k = marks.kinds; const clear = c => /rgba\(.*, 0\)$/.test(c) || c === 'transparent';
    check('patch notes', 'Fix (dot) and Adjust (diamond) marks are on the page' + tag, !!k.dot && !!k.diamond, JSON.stringify({ dot: k.dot && k.dot.n, diamond: k.diamond && k.diamond.n, up: k.up && k.up.n, down: k.down && k.down.n }));
    if (forced) {
      for (const kind of ['dot', 'diamond']) if (k[kind]) check('patch notes', `forced: ${kind} keeps a painted shape in the system text colour` + tag, k[kind].adjust === 'none' && k[kind].bg === marks.canvasText && k[kind].bg !== marks.canvas && k[kind].w > 3 && k[kind].h > 3, JSON.stringify(k[kind]));
      if (k.up) check('patch notes', 'forced: Buff arrow stays a triangle (clear sides) in the system text colour' + tag, k.up.adjust === 'none' && k.up.bottom === marks.canvasText && clear(k.up.left), JSON.stringify(k.up));
      if (k.down) check('patch notes', 'forced: Nerf arrow stays a triangle (clear sides) in the system text colour' + tag, k.down.adjust === 'none' && k.down.top === marks.canvasText && clear(k.down.left), JSON.stringify(k.down));
    } else {
      for (const kind of ['dot', 'diamond']) if (k[kind]) check('patch notes', `normal: ${kind} is drawn in the badge ink` + tag, !clear(k[kind].bg) && k[kind].bg === k[kind].color, JSON.stringify(k[kind]));
    }
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.waitForTimeout(200);
    await shot(page, 'patch-notes', width, forced);
    const openHero = page.locator('.pn-hero-card.is-open').first();
    if (await openHero.count()) { await openHero.scrollIntoViewIfNeeded(); await page.waitForTimeout(200); await page.screenshot({ path: `${SHOTS}/patch-notes-open-hero-${width}-${mode(forced)}.png` }); }
    for (const kind of ['dot', 'diamond', 'up']) {
      const el = page.locator(`.pn-verdict:has(.pn-mark-${kind})`).first();
      if (await el.count()) { await el.scrollIntoViewIfNeeded(); await el.screenshot({ path: `${SHOTS}/patch-mark-${kind}-${width}-${mode(forced)}.png` }); }
    }
    // Pixel check: the dot is not the same colour as the badge behind it.
    for (const kind of ['dot', 'diamond']) {
      const el = page.locator(`.pn-mark-${kind}`).first();
      if (!(await el.count())) continue;
      const buf = await el.screenshot({ type: 'png' });
      const distinct = await page.evaluate(async b64 => {
        const img = new Image(); img.src = 'data:image/png;base64,' + b64; await img.decode();
        const c = document.createElement('canvas'); c.width = img.width; c.height = img.height; const x = c.getContext('2d'); x.drawImage(img, 0, 0);
        const mid = x.getImageData(Math.floor(img.width / 2), Math.floor(img.height / 2), 1, 1).data;
        const bg = getComputedStyle(document.querySelector('.pn-verdict')).backgroundColor;
        return { mid: [...mid].slice(0, 3).join(','), w: img.width, h: img.height, bg };
      }, buf.toString('base64'));
      const bgRgb = (distinct.bg.match(/\d+/g) || []).slice(0, 3).join(',');
      check('patch notes', `${kind}: centre pixel differs from the badge fill` + tag, distinct.mid !== bgRgb, JSON.stringify(distinct));
    }
    await ctx.close();
  }
}

// ---------------------------------------------------------------------------
// Wiki: Edit is for crew members; a failed save names its cause.
// ---------------------------------------------------------------------------
if (want('wiki')) {
  const edit = 'button[id$="-edit"]';
  {
    const { ctx, page } = await ctxFor(1440, false, null);
    await open(page, '/wiki/tank-basics');
    check('wiki', 'guest: no Edit', await page.locator(edit).count() === 0);
    await ctx.close();
  }
  {
    await register('Visitor18');
    const { ctx, page } = await ctxFor(1440, false, 'Visitor18');
    await open(page, '/wiki/tank-basics');
    check('wiki', 'logged-in account with no membership: no Edit, History still there', await page.locator(edit).count() === 0 && await page.locator('button[aria-controls$="-history-panel"]').count() === 1);
    const direct = await page.evaluate(async () => (await fetch('/api/wiki/tank-basics', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ content_markdown: 'x', revision_note: null }) })).status);
    check('wiki', 'the API still answers 403 to that account', direct === 403, direct);
    await ctx.close();
  }
  for (const width of [1440, 390]) {
    const { ctx, page } = await ctxFor(width, false, 'Player01');
    await open(page, '/wiki/tank-basics');
    check('wiki', `member: Edit shows @${width}`, await page.locator(edit).count() === 1);
    await page.click(edit); await page.waitForSelector('.wiki-edit-form');
    const save = page.locator('.wiki-edit-buttons .btn.is-primary');
    const errText = async () => (await page.locator('.wiki-edit-form .field-error').textContent()).trim();
    // 403 from the server (stubbed with the server's own body shape).
    await page.route('**/api/wiki/tank-basics', r => r.request().method() === 'PUT' ? r.fulfill({ status: 403, contentType: 'application/json', body: '{"error":"Not an org member"}' }) : r.continue());
    await save.click(); await page.waitForTimeout(500);
    const e403 = await errText();
    check('wiki', `save 403 says members only, not "check your connection" @${width}`, e403 === "Couldn't save the page. Only crew members can edit the wiki.", e403);
    await page.screenshot({ path: `${SHOTS}/wiki-save-403-${width}-normal.png` });
    await page.unroute('**/api/wiki/tank-basics');
    // Network failure.
    await page.route('**/api/wiki/tank-basics', r => r.request().method() === 'PUT' ? r.abort('failed') : r.continue());
    await save.click(); await page.waitForTimeout(500);
    const eNet = await errText();
    check('wiki', `save with no network says check your connection @${width}`, eNet === "Couldn't save the page. Check your connection and try again.", eNet);
    await page.unroute('**/api/wiki/tank-basics');
    await page.locator('.wiki-edit-buttons .btn:not(.is-primary)').click();
    // History list keeps its list role.
    await page.click('button[aria-controls$="-history-panel"]'); await page.waitForTimeout(700);
    const list = await page.evaluate(() => { const u = document.querySelector('.wiki-revision-list'); return u ? { role: u.getAttribute('role'), n: u.children.length } : null; });
    check('wiki', `revision list has role=list @${width}`, !list || list.role === 'list', JSON.stringify(list));
    await ctx.close();
  }
  // Encoded slugs: a %2F segment stays inside its own API path.
  {
    const { ctx, page } = await ctxFor(1440, false, null);
    const seen = [];
    page.on('request', r => { const u = new URL(r.url()); if (u.pathname.startsWith('/api/')) seen.push(u.pathname); });
    await page.goto(B + '/wiki/..%252Fauth%252Fme', { waitUntil: 'load' }); await ready(page);
    await page.goto(B + '/blog/..%252Fauth%252Fme', { waitUntil: 'load' }); await ready(page);
    const wikiReq = seen.filter(p => p.startsWith('/api/wiki/')); const blogReq = seen.filter(p => p.startsWith('/api/articles/'));
    check('slugs', 'a wiki topic that decodes to ../auth/me is requested as one encoded segment', wikiReq.length > 0 && wikiReq.every(p => p.split('/').length === 4 || p.endsWith('/revisions')), JSON.stringify(wikiReq));
    check('slugs', 'a blog slug that decodes to ../auth/me is requested as one encoded segment', blogReq.length > 0 && blogReq.every(p => p.split('/').length === 4), JSON.stringify(blogReq));
    check('slugs', 'the missing article shows the not-found panel', /not found/i.test(await page.locator('h1').textContent()));
    await ctx.close();
  }
}

// ---------------------------------------------------------------------------
// Home: a member never sees the apply actions; hero panel over a page
// background; news card heading order.
// ---------------------------------------------------------------------------
if (want('home')) {
  const watch = () => {
    window.__applySeen = 0; window.__recruitSeen = 0;
    const scan = () => { const m = document.querySelector('.home-page'); if (!m) return; if (m.querySelector('a[href="/apply"]')) window.__applySeen++; if (m.querySelector('#recruit')) window.__recruitSeen++; };
    new MutationObserver(scan).observe(document, { childList: true, subtree: true });
  };
  for (const width of [1440, 390]) {
    {
      const { ctx, page } = await ctxFor(width, false, 'Player01');
      await ctx.addInitScript(watch);
      // Slow the session check so a flash would be caught.
      await page.route('**/api/auth/me', async r => { await sleep(700); await r.continue(); });
      await page.goto(B + '/', { waitUntil: 'load' }); await ready(page); await page.waitForTimeout(600);
      const m = await page.evaluate(() => ({ apply: window.__applySeen, recruit: window.__recruitSeen, hero: !!document.querySelector('.home-hero-title'), cls: document.querySelector('.home-page').className, cards: [...document.querySelectorAll('.home-news-card')].map(c => c.firstElementChild.tagName), metaAbove: (() => { const c = document.querySelector('.home-news-card'); if (!c) return null; return c.querySelector('.home-news-meta').getBoundingClientRect().top < c.querySelector('h3').getBoundingClientRect().top; })(), lists: [...document.querySelectorAll('.home-list, .home-rules')].map(l => l.getAttribute('role')) }));
      check('home', `member: "Join us" and the Recruit block never render, even with a slow session check @${width}`, m.hero && m.apply === 0 && m.recruit === 0, JSON.stringify({ apply: m.apply, recruit: m.recruit }));
      check('home', `page class has no dead alignment class @${width}`, !/align-/.test(m.cls), m.cls);
      check('home', `news cards start with their H3; the badge and date are still drawn above it @${width}`, m.cards.length > 0 && m.cards.every(t => t === 'H3') && m.metaAbove === true, JSON.stringify({ cards: m.cards, metaAbove: m.metaAbove }));
      check('home', `unstyled lists keep role=list @${width}`, m.lists.every(r => r === 'list'), JSON.stringify(m.lists));
      await shot(page, 'home-member', width, false);
      await ctx.close();
    }
    {
      const { ctx, page } = await ctxFor(width, false, null);
      await open(page, '/');
      const g = await page.evaluate(() => ({ apply: document.querySelectorAll('.home-page a[href="/apply"]').length, recruit: !!document.querySelector('#recruit'), heroBg: getComputedStyle(document.querySelector('.home-hero')).backgroundColor }));
      check('home', `guest: apply action and Recruit block are there @${width}`, g.apply >= 1 && g.recruit, JSON.stringify(g));
      check('home', `no page background: the hero has no fill @${width}`, /rgba\(0, 0, 0, 0\)|transparent/.test(g.heroBg), g.heroBg);
      await ctx.close();
    }
  }
  // Page background set: the hero gets an opaque --bg panel.
  {
    const settings = (await admin.req('GET', '/api/settings')).j;
    const set = await admin.req('PUT', '/api/settings', { ...settings, page_bg_color: '#7a1f1f' });
    if (set.status < 300) {
      for (const width of [1440, 390]) {
        const { ctx, page } = await ctxFor(width, false, null);
        await open(page, '/');
        const h = await page.evaluate(() => { const hero = document.querySelector('.home-hero'); const s = getComputedStyle(hero); const d = document.createElement('div'); d.style.color = 'var(--bg)'; document.body.appendChild(d); const bg = getComputedStyle(d).color; d.remove(); return { heroBg: s.backgroundColor, bg, pad: s.paddingLeft, body: getComputedStyle(document.body).backgroundColor, bodyAlt: getComputedStyle(document.querySelector('.site-shell, #app-root, body')).backgroundColor }; });
        check('home', `page background set: hero is an opaque --bg panel with side padding @${width}`, h.heroBg === h.bg && parseFloat(h.pad) > 0, JSON.stringify(h));
        const a = await shot(page, 'home-page-bg', width, false);
        const heroContrast = (a.contrast || []).filter(c => /home-hero/.test(JSON.stringify(c)));
        check('home', `page background set: no hero text under 4.5:1 @${width}`, heroContrast.length === 0, JSON.stringify(heroContrast).slice(0, 300));
        await ctx.close();
      }
      await admin.req('PUT', '/api/settings', { ...settings });
      const now = (await admin.req('GET', '/api/settings')).j;
      check('home', 'page background removed again', !now.page_bg_color, now.page_bg_color);
    } else check('home', 'could set a page background colour', false, set.status + ' ' + JSON.stringify(set.j).slice(0, 200));
  }
}

// ---------------------------------------------------------------------------
// Events: RSVP radios; a save survives paging away from its card.
// ---------------------------------------------------------------------------
if (want('events')) {
  const have = rows((await admin.req('GET', '/api/events')).j);
  const extra = [];
  for (let i = have.length; i < 12; i++) {
    const r = await admin.req('POST', '/api/events', { ...EVENT, title: `Test night ${String(i).padStart(2, '0')}`, day_of_week: i % 7, time: '20:00' });
    const id = r.j && (r.j.id || (r.j.data && r.j.data.id)); if (id) extra.push(id);
  }
  for (const width of [1440, 390]) {
    const { ctx, page } = await ctxFor(width, false, 'Player01');
    await open(page, '/events');
    await page.waitForSelector('[role=radiogroup]');
    const g = await page.evaluate(() => { const grp = document.querySelector('[role=radiogroup]'); const rs = [...grp.querySelectorAll('[role=radio]')]; return { name: grp.getAttribute('aria-label'), n: rs.length, tab: rs.map(r => r.tabIndex), checked: rs.map(r => r.getAttribute('aria-checked')), pressed: document.querySelectorAll('.event-rsvp [aria-pressed]').length, listRole: document.querySelector('.events-list').getAttribute('role'), h: rs.map(r => Math.round(r.getBoundingClientRect().height)) }; });
    check('events', `RSVP is a named radio group of three 44px radios, one Tab stop @${width}`, /^Your RSVP for /.test(g.name) && g.n === 3 && g.tab.filter(t => t === 0).length === 1 && g.pressed === 0 && g.h.every(h => h >= 44), JSON.stringify(g));
    check('events', `events list has role=list @${width}`, g.listRole === 'list');
    const first = page.locator('[role=radiogroup]').first().locator('[role=radio]');
    await first.nth(0).focus();
    let posts = 0; page.on('request', r => { if (r.method() === 'POST' && /\/rsvp$/.test(r.url())) posts++; });
    await page.keyboard.press('ArrowRight'); await page.waitForTimeout(150);
    const f1 = await page.evaluate(() => document.activeElement.textContent.trim());
    await page.keyboard.press('ArrowLeft'); await page.keyboard.press('ArrowLeft'); await page.waitForTimeout(150);
    const f2 = await page.evaluate(() => document.activeElement.textContent.trim());
    check('events', `arrow keys move focus between the radios and wrap, without saving @${width}`, f1 === 'Maybe' && /Can't make it/.test(f2) && posts === 0, `${f1} / ${f2} / posts ${posts}`);
    await page.keyboard.press('Home'); await page.keyboard.press('ArrowRight'); await page.keyboard.press('Space');
    const t = await waitToast(page, /RSVP: Maybe/);
    await page.waitForTimeout(400);
    const after = await page.evaluate(() => { const rs = [...document.querySelector('[role=radiogroup]').querySelectorAll('[role=radio]')]; return { checked: rs.map(r => r.getAttribute('aria-checked')), tab: rs.map(r => r.tabIndex), focus: document.activeElement.textContent.trim() }; });
    check('events', `Space answers: Maybe is checked, is the Tab stop and keeps focus; toast shown @${width}`, /RSVP: Maybe/.test(t) && after.checked.join() === 'false,true,false' && after.tab.join() === '-1,0,-1' && after.focus === 'Maybe' && posts === 1, JSON.stringify({ t, after, posts }));
    await shot(page, 'events-rsvp', width, false);
    // Paging away mid-save: the save is slowed, the pager moves on, and the
    // toast still arrives (and the answer is kept when coming back).
    await page.route('**/api/events/*/rsvp', async r => { await sleep(1500); await r.continue(); });
    await first.nth(0).click();
    await page.click('#events-pager-next');
    await page.waitForFunction(() => { const b = document.querySelector('#events-pager [aria-current="page"]'); return b && b.textContent.trim() === '2'; });
    const t2 = await waitToast(page, /RSVP: Going/, 6000);
    check('events', `a save still reports after the pager moved on @${width}`, /RSVP: Going/.test(t2), t2);
    await page.unroute('**/api/events/*/rsvp');
    // A failed save after paging away: error toast, and the old answer is back.
    await page.click('#events-pager-prev'); await page.waitForTimeout(500);
    const back = await page.evaluate(() => [...document.querySelector('[role=radiogroup]').querySelectorAll('[role=radio]')].map(r => r.getAttribute('aria-checked')).join());
    check('events', `the answer is still checked after coming back @${width}`, back === 'true,false,false', back);
    await page.route('**/api/events/*/rsvp', async r => { await sleep(1200); await r.fulfill({ status: 500, contentType: 'application/json', body: '{"error":"Internal server error"}' }); });
    await first.nth(2).click();
    await page.click('#events-pager-next'); await page.waitForTimeout(300);
    const t3 = await waitToast(page, /Couldn't save your RSVP/, 6000);
    await page.unroute('**/api/events/*/rsvp');
    await page.click('#events-pager-prev'); await page.waitForTimeout(500);
    const reverted = await page.evaluate(() => [...document.querySelector('[role=radiogroup]').querySelectorAll('[role=radio]')].map(r => r.getAttribute('aria-checked')).join());
    check('events', `a failed save after paging away shows its toast and puts the old answer back @${width}`, /Couldn't save your RSVP\. Try again in a moment\./.test(t3) && reverted === 'true,false,false', JSON.stringify({ t3, reverted }));
    await ctx.close();
  }
  for (const id of extra) await admin.req('DELETE', '/api/events/' + id);
}

// ---------------------------------------------------------------------------
// News: Pinned reads as its own word. Keys: cards keep their DOM node.
// ---------------------------------------------------------------------------
if (want('news')) {
  const { ctx, page } = await ctxFor(1440, false, null);
  await open(page, '/news');
  const h = await page.evaluate(() => { const h2 = [...document.querySelectorAll('.entry-row h2')].find(h => /Pinned/i.test(h.textContent)); return h2 ? h2.textContent : null; });
  check('news', 'pinned title has a space before the badge word', h && /\S Pinned$/i.test(h), JSON.stringify(h));
  await ctx.close();
}

// ---------------------------------------------------------------------------
// Trial applicant withdraws: the session ends, the page says "Log in".
// Runs last: it uses up Applicant01's application.
// ---------------------------------------------------------------------------
if (want('withdraw')) {
  const apps = rows((await admin.req('GET', '/api/applications')).j);
  const jar = new Jar(true); await jar.login('Applicant01');
  const mine = (await jar.req('GET', '/api/applications/mine')).j;
  const app = mine && (mine.id ? mine : mine.data);
  if (!app || !['pending', 'trial'].includes(app.status)) {
    check('withdraw', 'Applicant01 has an open application to test with', false, JSON.stringify(mine).slice(0, 200) + ' of ' + apps.length);
  } else {
    if (app.status === 'pending') {
      // Pending first: the confirm does not mention a trial.
      const { ctx, page } = await ctxFor(1440, false, 'Applicant01');
      await open(page, '/apply');
      await page.click('#withdraw-application-trigger'); await page.waitForSelector('.inline-confirm');
      const body = (await page.locator('.inline-confirm').textContent()).trim();
      check('withdraw', 'pending: the confirm does not mention a trial', !/trial/i.test(body), body);
      await ctx.close();
      const tr = await admin.req('PATCH', '/api/applications/' + app.id, { status: 'trial', review_notes: 'fix1 check' });
      check('withdraw', 'admin moved the application to trial', tr.status < 300, tr.status + ' ' + JSON.stringify(tr.j).slice(0, 160));
    }
    const { ctx, page } = await ctxFor(390, false, 'Applicant01');
    const bad = []; page.on('response', r => { if (r.status() >= 400) bad.push(`${r.status()} ${new URL(r.url()).pathname}`); });
    await open(page, '/apply');
    await page.click('#withdraw-application-trigger'); await page.waitForSelector('.inline-confirm');
    const body = (await page.locator('.inline-confirm').textContent()).trim();
    check('withdraw', 'trial: the confirm says it ends the trial and logs you out', /ends your trial and logs you out/.test(body), body);
    await page.screenshot({ path: `${SHOTS}/apply-withdraw-trial-390-normal.png`, fullPage: true });
    await page.locator('.inline-confirm__go').click();
    await page.waitForSelector('#apply-sign-in-heading', { timeout: 10000 }).catch(() => {});
    await page.waitForTimeout(800);
    const end = await page.evaluate(() => { const h = document.getElementById('apply-sign-in-heading'); return { heading: h && h.textContent.trim(), focused: document.activeElement === h, retry: [...document.querySelectorAll('main button')].filter(b => /Retry/.test(b.textContent)).length, status: [...document.querySelectorAll('main [role=status]')].map(s => s.textContent.trim()).filter(Boolean), login: !!document.querySelector('main a[href="/login"]'), active: document.activeElement.tagName }; });
    check('withdraw', 'after a trial withdraw the page shows "Log in to apply" with focus on it', end.heading === 'Log in to apply' && end.focused && end.login, JSON.stringify(end));
    check('withdraw', 'no dead Retry and no "Couldn\'t load your application"', end.retry === 0 && !end.status.some(s => /Couldn't load/.test(s)), JSON.stringify({ retry: end.retry, status: end.status, bad }));
    await page.screenshot({ path: `${SHOTS}/apply-after-trial-withdraw-390-normal.png`, fullPage: true });
    await ctx.close();
  }
}

await browser.close();
const fails = results.filter(r => !r.pass);
const reportPath = REPORT_DIR + '/report-fix1.json';
const prev = only && fs.existsSync(reportPath) ? JSON.parse(fs.readFileSync(reportPath, 'utf8')) : { results: [], audits: {} };
const groups = new Set(results.map(r => r.group));
const merged = { results: [...prev.results.filter(r => !groups.has(r.group)), ...results], audits: { ...prev.audits, ...audits } };
fs.writeFileSync(reportPath, JSON.stringify(merged, null, 1));
const auditFails = Object.entries(audits).filter(([, a]) => a.overflowX > 0 || a.unnamed.length || a.small.length || a.targets.length || a.dupIds.length || a.h1 !== 1 || a.mains !== 1 || (Array.isArray(a.contrast) && a.contrast.length));
console.log(`\n${results.length - fails.length} pass, ${fails.length} fail; audit on ${Object.keys(audits).length} full-page shots, ${auditFails.length} with findings`);
for (const [k, a] of auditFails) console.log('AUDIT', k, JSON.stringify(a).slice(0, 500));
for (const f of fails) console.log('FAILED', f.group + ': ' + f.name, f.detail);
process.exit(fails.length ? 1 : 0);
