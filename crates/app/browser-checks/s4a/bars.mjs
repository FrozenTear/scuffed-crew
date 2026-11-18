// Keyboard focus clear of the shell bars (QA 696 nit 1). Tabs forward
// through every stop on the public pages, then Shift+Tabs back, and fails on
// any stop in `main` or the footer whose box is not fully between the bottom
// of the sticky top bar and the top of the phone tab bar (or the bottom of
// the view where there is no tab bar). A box taller than that band cannot
// fit and is reported as info only.
//
// Run once per engine: `BROWSER=firefox node bars.mjs` and plain
// `node bars.mjs` (Chromium). Firefox does not scroll a focused element that
// is already partly visible, so without the shell's focusin guard its stops
// end partly under the bars. `GUARD_OFF=1` blocks that guard in the page (a
// capturing window listener stops `focusin` before it reaches the
// document), to show the same bundle failing without it.
// Optional filter: `node bars.mjs blog` runs only the pages whose path
// contains `blog`.
import fs from 'node:fs';
import path from 'node:path';
import { B, ENGINE, REPORT_DIR, launch, makeContext, open, sleep } from './lib.mjs';

const PAGES = ['/', '/blog', '/news', '/wiki/tank-basics', '/patch-notes', '/changelog/site'];
const WIDTHS = [1440, 390, 360];
const MAX_STOPS = 120;
const SLACK = 1; // px: sub-pixel layout, not a cover
const GUARD_OFF = process.env.GUARD_OFF === '1';
const filter = process.argv[2] || '';

const results = [];
const rec = (pass, name, detail = '') => {
  results.push({ pass, name, detail });
  console.log(`${pass === null ? 'INFO' : pass ? 'PASS' : 'FAIL'} ${name}${detail ? ': ' + detail : ''}`);
};

// Where the focused element is, against the bars, in viewport px.
// `walk` marks each stop, so a stop seen twice ends the walk.
const measure = walk => {
  const el = document.activeElement;
  if (!el || el === document.body) return { where: 'body' };
  if (!el.closest('main#main, footer.site-footer')) return { where: 'chrome' };
  if (el.dataset.barsWalk === walk) return { where: 'repeat' };
  el.dataset.barsWalk = walk;
  const top = document.querySelector('header.d1-top')?.getBoundingClientRect();
  const tab = document.querySelector('nav.site-tab-bar')?.getBoundingClientRect();
  const view = document.documentElement.clientHeight;
  const clearTop = top && top.height > 0 ? Math.max(0, top.bottom) : 0;
  const clearBottom = tab && tab.height > 0 ? Math.min(view, tab.top) : view;
  const r = el.getBoundingClientRect();
  const name = (el.getAttribute('aria-label') || el.textContent || el.tagName).trim().replace(/\s+/g, ' ').slice(0, 40);
  return { where: el.closest('footer.site-footer') ? 'footer' : 'main', name, tag: el.tagName.toLowerCase(), top: r.top, bottom: r.bottom, clearTop, clearBottom };
};

const browser = await launch();
try {
  for (const width of WIDTHS) {
    const ctx = await makeContext(browser, 'guest', width, 'dark');
    if (GUARD_OFF) {
      await ctx.addInitScript(() => window.addEventListener('focusin', e => e.stopPropagation(), true));
    }
    const page = await ctx.newPage();
    for (const p of PAGES.filter(p => p.includes(filter))) {
      await open(page, p);
      const label = `${ENGINE} ${width} ${p}`;
      let stops = 0, covered = 0, tall = 0;
      for (const key of ['Tab', 'Shift+Tab']) {
        // Forward from the top of the page; back from the end of it.
        await page.evaluate(back => {
          document.activeElement?.blur();
          window.scrollTo(0, back ? document.documentElement.scrollHeight : 0);
        }, key === 'Shift+Tab');
        let inPage = 0;
        for (let i = 0; i < MAX_STOPS; i++) {
          await page.keyboard.press(key);
          await sleep(80);
          await page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
          const m = await page.evaluate(measure, key);
          if (m.where === 'body' || m.where === 'repeat') break;
          if (m.where === 'chrome') {
            // Left the page for the bars after its stops: walk done.
            if (inPage > 0) break;
            continue;
          }
          inPage++;
          stops++;
          const height = m.bottom - m.top;
          const band = m.clearBottom - m.clearTop;
          if (height > band) {
            tall++;
            continue;
          }
          const under = Math.max(0, m.clearTop - m.top - SLACK) + Math.max(0, m.bottom - m.clearBottom - SLACK);
          if (under > 0) {
            covered++;
            rec(false, `${label} ${key}`, `${m.where} "${m.name}" ${m.top.toFixed(0)}-${m.bottom.toFixed(0)}, clear band ${m.clearTop.toFixed(0)}-${m.clearBottom.toFixed(0)}`);
          }
        }
      }
      if (tall) rec(null, label, `${tall} stop(s) taller than the clear band, not checked`);
      rec(stops > 0 && covered === 0, label, `${stops} stops, ${covered} partly under a bar or past the fold`);
    }
    await ctx.close();
  }
} finally {
  await browser.close();
}

const fails = results.filter(r => r.pass === false);
const out = path.join(REPORT_DIR, `report-bars-${ENGINE}${GUARD_OFF ? '-guard-off' : ''}.json`);
fs.writeFileSync(out, JSON.stringify({ base: B, engine: ENGINE, guardOff: GUARD_OFF, results }, null, 1));
console.log(`\n${results.filter(r => r.pass === true).length} pass, ${fails.length} fail (${ENGINE}${GUARD_OFF ? ', guard off' : ''}) -> ${out}`);
process.exit(fails.length ? 1 : 0);
