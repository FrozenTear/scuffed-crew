// WCAG 2.4.11 on /admin/settings (QA 706 nit 1): Tab through the whole page
// and check that no focused control is covered by the stuck Save bar or the
// phone tab bar. The walk runs twice per size: with the bar at its normal
// height, and after a failed save (stubbed 500) has put an error line in the
// bar and made it taller (QA 706 fix4). The failed-save walk goes both ways
// (Tab, then Shift+Tab back). Then (QA 706 nit 2) Tab along the admin sub-nav
// on phone and check that each focused link is fully inside the sub-nav and
// clear of its edge fade. Last, a mouse click on a checkbox that straddles
// the bar must not scroll the page (only keyboard focus is moved clear).
// QA 706 FIX5: a save that cannot reach the server (the `PUT` is aborted, as
// a stopped server does) must show one short sentence, never the browser's
// raw error (Chromium put a 1,106 character stack in the bar), and leave every
// focused control clear; a very long server reason is clamped to three lines
// with the full text still in the line.
// Run once per engine: BROWSER=chromium and BROWSER=firefox.
import { BROWSER, launch, makeContext, open, reporter, settled } from './lib.mjs';

const { rec, finish } = reporter('focus-' + BROWSER);
// 360 at 844 tall too: QA measured Firefox there.
const SIZES = [[1440, 900], [390, 844], [360, 844], [360, 740]];
const MAX_STOPS = 150;
const FIRST_STOP = '.admin-settings-page a, .admin-settings-page button';

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
    barHeight: bar ? Math.round(bar.getBoundingClientRect().height) : null,
    tabTop: Math.round(tabTop),
    nav: nav ? (() => { const n = nav.getBoundingClientRect(); return { left: Math.round(n.left), right: Math.round(n.right), start: nav.classList.contains('has-more-start'), end: nav.classList.contains('has-more-end'), fade }; })() : null,
  };
};

// Tab (or Shift+Tab) from `startSel` until `stopId` has focus. Returns the
// covered stops, the tightest stop and whether the end was reached.
async function walk(page, key, startSel, endSel) {
  const covered = []; let stops = 0; let tightest = null; let reached = false; let barHeight = null;
  await page.evaluate(([sel, top]) => {
    if (top) scrollTo(0, 0);
    document.querySelector(sel).focus();
  }, [startSel, startSel === FIRST_STOP]);
  for (let i = 0; i < MAX_STOPS; i++) {
    await page.keyboard.press(key);
    // QA nit 6: presses 3 ms apart give off-screen artifacts in Firefox.
    await page.waitForTimeout(80);
    const p = await page.evaluate(probe);
    if (!p) continue;
    stops++;
    barHeight = Math.max(barHeight ?? 0, p.barHeight ?? 0);
    if (!p.inBar) {
      const gap = p.barTop - p.bottom;
      if (tightest === null || gap < tightest.gap) tightest = { gap, ...p };
      if (p.bottom > p.barTop) covered.push(`${p.id} ${p.top}-${p.bottom} bar ${p.barTop}`);
      if (p.bottom > p.tabTop) covered.push(`${p.id} ${p.top}-${p.bottom} tab bar ${p.tabTop}`);
    }
    if (await page.evaluate(sel => document.activeElement === document.querySelector(sel), endSel)) { reached = true; break; }
  }
  const t = tightest ? `tightest ${tightest.id} ${tightest.top}-${tightest.bottom} vs bar ${tightest.barTop}` : 'no stops';
  return { covered, stops, reached, t, barHeight };
}

// Make Save fail (any 4xx/5xx or network error takes the same path), press it
// from the keyboard and wait for the error line in the bar. `fail` answers
// the PUT; `expect` is a part of the line that shows once it has failed.
const STUB_500 = route => route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Stubbed failure for the focus check: the settings could not be stored' }) });
async function failSave(page, fail = STUB_500, expect = 'Stubbed failure') {
  await page.unroute('**/api/settings');
  await page.route('**/api/settings', route => route.request().method() === 'PUT' ? fail(route) : route.continue());
  await page.focus('#settings-save');
  await page.keyboard.press('Enter');
  await page.waitForFunction(want => {
    const s = document.querySelector('#settings-save-status');
    return s && s.classList.contains('is-error') && s.textContent.includes(want);
  }, expect, { timeout: 10000 });
  await page.waitForTimeout(300);
}

// The bar's live line: its text, and its box against its own line height.
const statusLine = () => {
  const s = document.querySelector('#settings-save-status');
  const cs = getComputedStyle(s);
  const lh = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.5;
  return { text: s.textContent, height: Math.round(s.getBoundingClientRect().height), lines: s.getBoundingClientRect().height / lh, lh, clipped: s.scrollHeight > s.clientHeight + 1 };
};

const NETWORK_LINE = 'Could not save settings. Could not reach the server. Check your connection and try again.';
const RAW_BITS = ['JsValue', 'TypeError', 'Failed to fetch', 'NetworkError', 'http://', '.js:', 'Network error'];

// The page's scroll-padding-bottom against the stuck bar's real top: the
// reserved strip must end exactly where the bar starts. The computed value
// can stay an unresolved calc() (`0%` off phone), so it is measured by what
// it does: a 1 px marker in the form, scrolled to the end of the view, stops
// on the padding line.
const reserved = () => {
  const form = document.querySelector('.form-stack');
  const mark = document.createElement('div');
  mark.style.cssText = 'position:absolute;left:0;width:1px;height:1px;top:' + Math.round(form.offsetHeight / 2) + 'px';
  const was = form.style.position;
  form.style.position = 'relative';
  form.appendChild(mark);
  mark.scrollIntoView({ block: 'end' });
  const padLine = Math.round(mark.getBoundingClientRect().bottom);
  mark.remove();
  form.style.position = was;
  const bar = document.querySelector('.settings-save-bar').getBoundingClientRect();
  return { barTop: Math.round(bar.top), barHeight: Math.round(bar.height), padLine, pad: Math.round(innerHeight - padLine) };
};

const browser = await launch();
for (const [width, height] of SIZES) {
  const ctx = await makeContext(browser, 'admin', width);
  const page = await ctx.newPage();
  await page.setViewportSize({ width, height });
  await open(page, '/admin/settings');
  await settled(page);
  await page.waitForSelector('#settings-save');

  // The form at the bar's normal height: header link to Save.
  const normal = await walk(page, 'Tab', FIRST_STOP, '#settings-save');
  console.log(`  ${BROWSER} ${width}x${height} normal bar ${normal.barHeight}px: ${normal.stops} stops, ${normal.t}`);
  rec(`${BROWSER} ${width}x${height}: /admin/settings, no focused control under the Save bar or tab bar (${normal.stops} stops; ${normal.t})`,
    normal.reached && normal.stops > 20 && normal.covered.length === 0, (normal.reached ? '' : 'Save never took focus | ') + normal.covered.join(' | '));

  // The same walks after a failed save: the error line makes the bar taller.
  await failSave(page);
  const res = await page.evaluate(reserved);
  console.log(`  ${BROWSER} ${width}x${height} failed save: bar ${res.barHeight}px (was ${normal.barHeight}), top ${res.barTop}, scroll-padding-bottom ${res.pad} (line at ${res.padLine})`);
  rec(`${BROWSER} ${width}x${height}: after a failed save the bar is taller (${normal.barHeight} -> ${res.barHeight}px)`, res.barHeight > normal.barHeight);
  rec(`${BROWSER} ${width}x${height}: after a failed save, scroll-padding-bottom ends at the bar's top (line ${res.padLine}, bar ${res.barTop})`,
    Math.abs(res.padLine - res.barTop) <= 1);
  const down = await walk(page, 'Tab', FIRST_STOP, '#settings-save');
  console.log(`  ${BROWSER} ${width}x${height} failed save, Tab: ${down.stops} stops, ${down.t}`);
  rec(`${BROWSER} ${width}x${height}: failed save, Tab: no focused control under the Save bar or tab bar (${down.stops} stops; ${down.t})`,
    down.reached && down.stops > 20 && down.covered.length === 0, (down.reached ? '' : 'Save never took focus | ') + down.covered.join(' | '));
  const up = await walk(page, 'Shift+Tab', '#settings-save', FIRST_STOP);
  console.log(`  ${BROWSER} ${width}x${height} failed save, Shift+Tab: ${up.stops} stops, ${up.t}`);
  rec(`${BROWSER} ${width}x${height}: failed save, Shift+Tab: no focused control under the Save bar or tab bar (${up.stops} stops; ${up.t})`,
    up.reached && up.stops > 20 && up.covered.length === 0, (up.reached ? '' : 'the header link never took focus | ') + up.covered.join(' | '));
  await ctx.close();

  // The sub-nav (phone): start on its first link, Tab along it.
  if (width < 820) {
    const ctx2 = await makeContext(browser, 'admin', width);
    const page2 = await ctx2.newPage();
    await page2.setViewportSize({ width, height });
    await open(page2, '/admin/settings');
    await settled(page2);
    const cut = []; let navStops = 0;
    await page2.evaluate(() => { scrollTo(0, 0); const n = document.querySelector('.admin-subnav'); n.scrollLeft = 0; n.querySelector('a').focus(); });
    for (let i = 0; i < 40; i++) {
      await page2.keyboard.press('Tab');
      await page2.waitForTimeout(80);
      const p = await page2.evaluate(probe);
      if (!p || !p.nav) break;
      navStops++;
      // Fully inside the strip, and out of a fade that is showing.
      const minL = p.nav.left + (p.nav.start ? p.nav.fade : 0) - 1;
      const maxR = p.nav.right - (p.nav.end ? p.nav.fade : 0) + 1;
      if (p.left < minL || p.right > maxR) cut.push(`${p.id} ${p.left}-${p.right} in ${p.nav.left}-${p.nav.right} start=${p.nav.start} end=${p.nav.end}`);
    }
    rec(`${BROWSER} ${width}x${height}: each focused admin sub-nav link is fully in view (${navStops} links)`, navStops > 5 && cut.length === 0, cut.join(' | '));
    await ctx2.close();
  }
}

// QA 706 FIX5: the server cannot be reached. The PUT is aborted (what a
// stopped server or a dropped connection gives the page), then the walks
// run with the bar showing that failure. Last, a 400 with a 1,200 character
// reason with no spaces: the line stops at three lines, all text kept.
for (const [width, height] of SIZES) {
  const ctx = await makeContext(browser, 'admin', width);
  const page = await ctx.newPage();
  await page.setViewportSize({ width, height });
  await open(page, '/admin/settings');
  await settled(page);
  await page.waitForSelector('#settings-save');
  const normalBar = await page.evaluate(() => Math.round(document.querySelector('.settings-save-bar').getBoundingClientRect().height));

  await failSave(page, route => route.abort('failed'), 'Could not save settings');
  const line = await page.evaluate(statusLine);
  const res = await page.evaluate(reserved);
  const leaks = RAW_BITS.filter(bit => line.text.includes(bit));
  console.log(`  ${BROWSER} ${width}x${height} unreachable: "${line.text}" (${line.text.length} chars), bar ${res.barHeight}px (was ${normalBar}), line ${res.padLine} vs bar ${res.barTop}`);
  rec(`${BROWSER} ${width}x${height}: server unreachable, the Save bar reads the short message (${line.text.length} chars)`,
    line.text === NETWORK_LINE && leaks.length === 0, `got "${line.text.slice(0, 200)}" leaks ${leaks.join(',')}`);
  rec(`${BROWSER} ${width}x${height}: server unreachable, the bar stays small (${normalBar} -> ${res.barHeight}px, line ${line.lines.toFixed(2)} lines) and scroll-padding-bottom ends at its top (line ${res.padLine}, bar ${res.barTop})`,
    line.lines <= 3.05 && res.barHeight <= normalBar + 3 * line.lh + 2 && Math.abs(res.padLine - res.barTop) <= 1);
  const down = await walk(page, 'Tab', FIRST_STOP, '#settings-save');
  console.log(`  ${BROWSER} ${width}x${height} unreachable, Tab: ${down.stops} stops, ${down.t}`);
  rec(`${BROWSER} ${width}x${height}: server unreachable, Tab: no focused control under the Save bar or tab bar (${down.stops} stops; ${down.t})`,
    down.reached && down.stops > 20 && down.covered.length === 0, (down.reached ? '' : 'Save never took focus | ') + down.covered.join(' | '));
  const up = await walk(page, 'Shift+Tab', '#settings-save', FIRST_STOP);
  console.log(`  ${BROWSER} ${width}x${height} unreachable, Shift+Tab: ${up.stops} stops, ${up.t}`);
  rec(`${BROWSER} ${width}x${height}: server unreachable, Shift+Tab: no focused control under the Save bar or tab bar (${up.stops} stops; ${up.t})`,
    up.reached && up.stops > 20 && up.covered.length === 0, (up.reached ? '' : 'the header link never took focus | ') + up.covered.join(' | '));

  // A very long server reason: clamped to three lines, the text all there.
  const reason = 'LongReason' + 'x'.repeat(1190);
  await failSave(page, route => route.fulfill({ status: 400, contentType: 'application/json', body: JSON.stringify({ error: reason }) }), 'LongReason');
  const long = await page.evaluate(statusLine);
  const lres = await page.evaluate(reserved);
  console.log(`  ${BROWSER} ${width}x${height} long reason: ${long.text.length} chars, ${long.lines.toFixed(2)} lines shown, clipped ${long.clipped}, bar ${lres.barHeight}px`);
  rec(`${BROWSER} ${width}x${height}: a 1,200 character reason is clamped to 3 lines, full text kept (${long.lines.toFixed(2)} lines, bar ${lres.barHeight}px)`,
    long.lines <= 3.05 && long.clipped && long.text.includes(reason) && Math.abs(lres.padLine - lres.barTop) <= 1,
    `lines ${long.lines} clipped ${long.clipped} len ${long.text.length} line ${lres.padLine} bar ${lres.barTop}`);
  const ldown = await walk(page, 'Tab', FIRST_STOP, '#settings-save');
  rec(`${BROWSER} ${width}x${height}: long reason, Tab: no focused control under the Save bar or tab bar (${ldown.stops} stops; ${ldown.t})`,
    ldown.reached && ldown.stops > 20 && ldown.covered.length === 0, (ldown.reached ? '' : 'Save never took focus | ') + ldown.covered.join(' | '));
  await ctx.close();
}

// A mouse click on a checkbox that straddles the bar's top: the press
// focuses it, but it is not keyboard focus, so the page must not scroll out
// from under the pointer. The click still lands (the box toggles).
for (const [width, height] of [[1440, 900], [360, 844]]) {
  const ctx = await makeContext(browser, 'admin', width);
  const page = await ctx.newPage();
  await page.setViewportSize({ width, height });
  await open(page, '/admin/settings');
  await settled(page);
  await page.waitForSelector('#settings-save');
  const spot = await page.evaluate(() => {
    const bar = document.querySelector('.settings-save-bar');
    const box = document.querySelector('#settings-section-ethos');
    box.scrollIntoView({ block: 'center' });
    const r0 = box.getBoundingClientRect();
    // Put the box's top 8 px above the bar's top: the rest is under the bar.
    scrollBy(0, Math.round(r0.top - (bar.getBoundingClientRect().top - 8)));
    const r = box.getBoundingClientRect();
    return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + 3), top: Math.round(r.top), bottom: Math.round(r.bottom), barTop: Math.round(bar.getBoundingClientRect().top), scrollY: scrollY, checked: box.checked };
  });
  await page.mouse.click(spot.x, spot.y);
  await page.waitForTimeout(300);
  const after = await page.evaluate(() => ({ scrollY, focused: document.activeElement && document.activeElement.id, checked: document.querySelector('#settings-section-ethos').checked }));
  const moved = Math.round(after.scrollY - spot.scrollY);
  console.log(`  ${BROWSER} ${width}x${height} click: box ${spot.top}-${spot.bottom} vs bar ${spot.barTop}, page moved ${moved}px, focused ${after.focused}, toggled ${after.checked !== spot.checked}`);
  rec(`${BROWSER} ${width}x${height}: a mouse click on a checkbox that straddles the Save bar does not scroll the page (moved ${moved}px)`,
    spot.top < spot.barTop && spot.bottom > spot.barTop && moved === 0 && after.checked !== spot.checked,
    `box ${spot.top}-${spot.bottom} bar ${spot.barTop} moved ${moved} toggled ${after.checked !== spot.checked}`);
  await ctx.close();
}
await browser.close();
finish();
