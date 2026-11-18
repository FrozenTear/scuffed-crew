// WCAG 2.4.11 on /admin/settings (QA 706 nit 1): Tab through the whole page
// and check that no focused control is covered by the stuck Save bar or the
// phone tab bar. Then (QA 706 nit 2) Tab along the admin sub-nav on phone and
// check that each focused link is fully inside the sub-nav and clear of its
// edge fade. Run once per engine: BROWSER=chromium and BROWSER=firefox.
import { BROWSER, launch, makeContext, open, reporter, settled } from './lib.mjs';

const { rec, finish } = reporter('focus-' + BROWSER);
// 360 at 844 tall too: QA measured Firefox there.
const SIZES = [[1440, 900], [390, 844], [360, 844], [360, 740]];
const MAX_STOPS = 150;

// Where the focused element is, against the bars that can cover it.
const probe = () => {
  const e = document.activeElement;
  if (!e || e === document.body) return null;
  const r = e.getBoundingClientRect();
  const bar = document.querySelector('.settings-save-bar');
  const tab = document.querySelector('.site-tab-bar');
  const tabTop = tab && getComputedStyle(tab).display !== 'none' ? tab.getBoundingClientRect().top : innerHeight;
  const nav = e.closest('.admin-subnav');
  const fade = nav ? parseFloat(getComputedStyle(nav).getPropertyValue('--space-10')) || 40 : 0;
  return {
    id: e.id || (e.textContent || '').trim().slice(0, 30) || e.tagName,
    top: Math.round(r.top), bottom: Math.round(r.bottom), left: Math.round(r.left), right: Math.round(r.right),
    inBar: !!(bar && bar.contains(e)),
    barTop: bar ? Math.round(bar.getBoundingClientRect().top) : null,
    tabTop: Math.round(tabTop),
    nav: nav ? (() => { const n = nav.getBoundingClientRect(); return { left: Math.round(n.left), right: Math.round(n.right), start: nav.classList.contains('has-more-start'), end: nav.classList.contains('has-more-end'), fade }; })() : null,
  };
};

const browser = await launch();
for (const [width, height] of SIZES) {
  const ctx = await makeContext(browser, 'admin', width);
  const page = await ctx.newPage();
  await page.setViewportSize({ width, height });
  await open(page, '/admin/settings');
  await settled(page);
  await page.waitForSelector('#settings-save');
  const covered = []; const cut = []; let stops = 0; let tightest = null; let navStops = 0; let reachedSave = false;
  // The form: start on the header's first link, Tab until Save has focus.
  await page.evaluate(() => { scrollTo(0, 0); document.querySelector('.admin-settings-page a, .admin-settings-page button').focus(); });
  for (let i = 0; i < MAX_STOPS; i++) {
    await page.keyboard.press('Tab');
    // QA nit 6: presses 3 ms apart give off-screen artifacts in Firefox.
    await page.waitForTimeout(80);
    const p = await page.evaluate(probe);
    if (!p) continue;
    stops++;
    if (!p.inBar) {
      const gap = p.barTop - p.bottom;
      if (tightest === null || gap < tightest.gap) tightest = { gap, ...p };
      if (p.bottom > p.barTop) covered.push(`${p.id} ${p.top}-${p.bottom} bar ${p.barTop}`);
      if (p.bottom > p.tabTop) covered.push(`${p.id} ${p.top}-${p.bottom} tab bar ${p.tabTop}`);
    }
    if (p.id === 'settings-save') { reachedSave = true; break; }
  }
  // The sub-nav (phone): start on its first link, Tab along it.
  if (width < 820) {
    await page.evaluate(() => { scrollTo(0, 0); const n = document.querySelector('.admin-subnav'); n.scrollLeft = 0; n.querySelector('a').focus(); });
    for (let i = 0; i < 40; i++) {
      await page.keyboard.press('Tab');
      await page.waitForTimeout(80);
      const p = await page.evaluate(probe);
      if (!p || !p.nav) break;
      navStops++;
      // Fully inside the strip, and out of a fade that is showing.
      const minL = p.nav.left + (p.nav.start ? p.nav.fade : 0) - 1;
      const maxR = p.nav.right - (p.nav.end ? p.nav.fade : 0) + 1;
      if (p.left < minL || p.right > maxR) cut.push(`${p.id} ${p.left}-${p.right} in ${p.nav.left}-${p.nav.right} start=${p.nav.start} end=${p.nav.end}`);
    }
  }
  const t = tightest ? `tightest ${tightest.id} ${tightest.top}-${tightest.bottom} vs bar ${tightest.barTop}` : 'no stops';
  console.log(`  ${BROWSER} ${width}x${height}: ${stops} stops, ${t}`);
  rec(`${BROWSER} ${width}x${height}: /admin/settings, no focused control under the Save bar or tab bar (${stops} stops; ${t})`, reachedSave && stops > 20 && covered.length === 0, (reachedSave ? '' : 'Save never took focus | ') + covered.join(' | '));
  if (width < 820) rec(`${BROWSER} ${width}x${height}: each focused admin sub-nav link is fully in view (${navStops} links)`, navStops > 5 && cut.length === 0, cut.join(' | '));
  await ctx.close();
}
await browser.close();
finish();
