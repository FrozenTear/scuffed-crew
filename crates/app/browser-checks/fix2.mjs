// FIX2 (PR 693): on /strategy/editor Tab moves focus as on any page. The Tab
// panel toggle fires only while the map canvas holds focus; Esc and Shift+Tab
// leave it. Runs on a saved strategy with a map (seed-editor.mjs), so no map
// picker opens on load and the map is fixed (issue 817); the map change runs
// on a new strategy. The picker modal itself is covered in keys.mjs.
import { launch, makeContext, open, readState, reporter } from './lib.mjs';

const { rec, finish } = reporter('fix2');
const EDITOR = '/strategy/editor/' + readState('strategy.json').id;
const info = page => page.evaluate(() => {
  const a = document.activeElement; if (!a) return null;
  const zone = a.classList.contains('skip-link') ? 'skip' : a.closest('.canvas-container') ? 'canvas' : a.closest('.d1-top') ? 'top' : a.closest('.site-tab-bar') ? 'tabbar' : a.closest('.site-footer') ? 'footer' : a.closest('.map-picker-overlay') ? 'picker' : a.closest('#main') ? 'main' : a === document.body ? 'body' : 'other';
  const cv = document.querySelector('.map-canvas-overlay');
  const afterCanvas = !!cv && !!(cv.compareDocumentPosition(a) & Node.DOCUMENT_POSITION_FOLLOWING) && a !== document.body;
  return { zone, afterCanvas, n: [...document.querySelectorAll('*')].indexOf(a), tag: a.tagName, cls: String(a.className).slice(0, 40), id: a.id, text: (a.textContent || '').trim().slice(0, 20) };
});
// `hint`: the live hint node is always mounted (Vision nit 9); it counts as
// shown while it has text.
const panels = page => page.evaluate(() => ({
  left: !document.querySelector('.editor-sidebar-left').classList.contains('collapsed'),
  right: !document.querySelector('.editor-sidebar-right').classList.contains('collapsed'),
  bottom: !document.querySelector('.element-list-wrapper').classList.contains('collapsed'),
  hint: !!(document.querySelector('.canvas-key-hint')?.textContent || '').trim(),
  tool: [...document.querySelectorAll('.tb-tool-btn')].findIndex(b => b.classList.contains('active')),
}));
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
// A point where a click really lands on the overlay canvas (panels may cover parts of it).
const canvasPoint = page => page.evaluate(() => {
  const c = document.querySelector('.map-canvas-overlay'); const r = c.getBoundingClientRect();
  for (let fy = 0.5; fy > 0.05; fy -= 0.05) for (let fx = 0.5; fx < 0.96; fx += 0.05) {
    const x = Math.round(r.left + r.width * fx), y = Math.round(r.top + r.height * fy);
    if (document.elementFromPoint(x, y) === c) return { x, y };
  }
  return null;
});
const browser = await launch();
for (const role of ['guest', 'member', 'admin']) {
  for (const width of [1440, 390]) {
    const T = `${role} ${width}`;
    const ctx = await makeContext(browser, role, width); const page = await ctx.newPage();
    const errors = []; page.on('pageerror', e => errors.push(String(e).slice(0, 120)));
    await open(page, EDITOR);
    if (!(await page.$('.editor-container'))) { rec(`${T}: editor renders`, false, page.url()); await ctx.close(); continue; }
    rec(`${T}: a saved strategy with a map opens with no picker`, !(await page.$('.map-picker-overlay')) && !(await page.$('.map-selector-overlay')));

    // ---- 1. Tab from page load: skip link, then the nav, then on through the page.
    const atLoad = await panels(page);
    await page.keyboard.press('Tab'); let a = await info(page);
    const vis = await page.evaluate(() => { const r = document.activeElement.getBoundingClientRect(); return r.top >= 0 && r.left >= 0 && r.bottom <= innerHeight && r.width > 0; });
    rec(`${T}: first Tab from page load lands on the skip link, visible`, a.zone === 'skip' && vis, JSON.stringify(a));
    const seq = [a];
    for (let i = 0; i < 260; i++) {
      await page.keyboard.press('Tab'); a = await info(page); seq.push(a);
      if (a.zone === 'body' || a.zone === 'skip') break; // wrapped past the last stop
    }
    const zones = seq.map(s => s.zone).filter((z, i, all) => i === 0 || all[i - 1] !== z);
    rec(`${T}: second Tab is in the site nav (top bar)`, seq[1].zone === 'top', JSON.stringify(seq[1]));
    const topStops = seq.filter(s => s.zone === 'top').length;
    rec(`${T}: Tab walks the top bar (${topStops} stops) and then enters the page`, topStops >= 2 && zones.indexOf('main') > zones.indexOf('top'), zones.join(' > '));
    const stalled = seq.findIndex((s, i) => i > 0 && s.n === seq[i - 1].n && s.zone !== 'body');
    rec(`${T}: no Tab press leaves focus where it was (${seq.length} presses)`, stalled === -1, stalled === -1 ? '' : JSON.stringify(seq[stalled]));
    rec(`${T}: the map canvas is not a Tab stop`, !seq.some(s => s.tag === 'CANVAS'), '');
    const past = seq.filter(s => s.afterCanvas).length;
    rec(`${T}: Tab gets past the map (${past} stops after it) and wraps`, past >= 3 && ['body', 'skip'].includes(seq[seq.length - 1].zone), zones.join(' > '));
    if (width === 390) rec(`${T}: Tab reaches the phone tab bar`, zones.includes('tabbar'), zones.join(' > '));
    const st0 = await panels(page);
    rec(`${T}: tabbing through the page toggled no panel`, same(st0, atLoad) && !st0.hint, JSON.stringify(st0));

    // ---- 2. Skip link works on the editor.
    await open(page, EDITOR);
    await page.keyboard.press('Tab'); await page.keyboard.press('Enter'); await page.waitForTimeout(100);
    a = await info(page);
    rec(`${T}: skip link lands on #main`, a.id === 'main' && a.tag === 'MAIN', JSON.stringify(a));
    await page.keyboard.press('Shift+Tab'); a = await info(page);
    rec(`${T}: Shift+Tab from #main goes back to the top bar`, a.zone === 'top', JSON.stringify(a));

    // ---- 3. The map of a saved strategy is fixed (issue 817: the update
    // API takes no map). It is plain text that says so, not a button, and a
    // click on it opens no picker.
    const fixedMap = await page.evaluate(() => { const m = document.getElementById('editor-map-button'); return m && { tag: m.tagName, tabindex: m.getAttribute('tabindex'), text: m.textContent.trim() }; });
    rec(`${T}: a saved strategy's map is text "Map: ... (fixed once saved)", not a button`, !!fixedMap && fixedMap.tag === 'SPAN' && fixedMap.tabindex === '-1' && /^Map: .+ \(fixed once saved\)$/.test(fixedMap.text), JSON.stringify(fixedMap));
    await page.click('#editor-map-button'); await page.waitForTimeout(300);
    rec(`${T}: a click on the fixed map opens no picker`, !(await page.$('#map-picker')) && !(await page.$('.map-picker-overlay')));
    // Changing the map is for a new strategy only. In its own page, closed
    // without running beforeunload, so its unsaved pick prompts nothing.
    {
      const fresh = await ctx.newPage();
      await open(fresh, '/strategy/editor');
      rec(`${T}: a new strategy opens the picker`, !!(await fresh.$('#map-picker[role="dialog"]')));
      await fresh.click('.map-picker-item >> nth=1'); await fresh.waitForTimeout(500);
      const n = await info(fresh);
      const btn = await fresh.evaluate(() => { const m = document.getElementById('editor-map-button'); return m && { tag: m.tagName, text: m.textContent.trim() }; });
      rec(`${T}: picking a map closes the picker, focus back on the Map button`, !(await fresh.$('.map-picker-overlay')) && n.id === 'editor-map-button' && !!btn && btn.tag === 'BUTTON' && !btn.text.includes('('), JSON.stringify({ n, btn }));
      await fresh.click('#editor-map-button'); await fresh.waitForTimeout(300);
      rec(`${T}: before the first save the Map button opens the picker again`, !!(await fresh.$('#map-picker[role="dialog"]')));
      await fresh.close();
    }
    await page.bringToFront();
    if (width === 390) {
      // Known gap from S2: with both side panels open the phone canvas is 0px wide. Close them.
      await page.click('.editor-sidebar-left .sidebar-collapse-btn');
      // Shortcuts act only in the map canvas, so close the right one by its button too.
      await page.click('.editor-sidebar-right .sidebar-collapse-btn'); await page.waitForTimeout(200);
    }
    let pt = await canvasPoint(page);
    if (!pt) { rec(`${T}: canvas has a clickable point`, false); await ctx.close(); continue; }
    await page.mouse.click(pt.x, pt.y); await page.waitForTimeout(100);
    a = await info(page);
    rec(`${T}: a click focuses the map canvas`, a.tag === 'CANVAS' && a.zone === 'canvas', JSON.stringify(a));
    let before = await panels(page);
    rec(`${T}: Vision nit 20, a mouse focus shows no key hint`, !before.hint, JSON.stringify(before));
    await page.keyboard.press('Tab'); await page.waitForTimeout(150);
    let after = await panels(page); a = await info(page);
    rec(`${T}: Tab inside the canvas toggles the left panel, focus stays on the canvas`, after.left === !before.left && after.right === before.right && after.bottom === before.bottom && a.tag === 'CANVAS', JSON.stringify({ before, after, a }));
    rec(`${T}: the key hint shows after the toggle`, after.hint && await page.evaluate(() => { const h = document.querySelector('.canvas-key-hint'); const r = h.getBoundingClientRect(); return r.width > 0 && r.left >= 0 && r.right <= innerWidth && h.textContent.includes('Esc: cancel, then leave the map'); }));
    await page.keyboard.press('Tab'); await page.waitForTimeout(150);
    let again = await panels(page); a = await info(page);
    rec(`${T}: a second Tab toggles it back`, again.left === before.left && a.tag === 'CANVAS', JSON.stringify(again));

    // Esc leaves (nothing in progress: no drawing, no selection).
    before = again;
    await page.keyboard.press('Escape'); await page.waitForTimeout(150);
    a = await info(page); after = await panels(page);
    rec(`${T}: Esc leaves the canvas (focus on #main), no panel toggled, hint gone`, a.tag === 'MAIN' && a.id === 'main' && after.left === before.left && after.right === before.right && !after.hint, JSON.stringify({ a, after }));
    await page.keyboard.press('Tab'); await page.waitForTimeout(100);
    const b = await info(page); const afterTab = await panels(page);
    rec(`${T}: after Esc, Tab moves focus on and toggles nothing`, b.tag !== 'CANVAS' && b.tag !== 'MAIN' && b.zone === 'main' && same({ ...afterTab, hint: 0 }, { ...after, hint: 0 }), JSON.stringify(b));

    // Shift+Tab leaves.
    pt = await canvasPoint(page); await page.mouse.click(pt.x, pt.y); await page.waitForTimeout(100);
    a = await info(page); before = await panels(page);
    const canvasN = a.n;
    await page.keyboard.press('Shift+Tab'); await page.waitForTimeout(150);
    const c = await info(page); after = await panels(page);
    const precedes = await page.evaluate(() => !!(document.querySelector('.map-canvas-overlay').compareDocumentPosition(document.activeElement) & Node.DOCUMENT_POSITION_PRECEDING));
    rec(`${T}: canvas focused again by click`, a.tag === 'CANVAS');
    rec(`${T}: Shift+Tab leaves the canvas to the previous stop, no panel toggled`, c.tag !== 'CANVAS' && c.zone === 'main' && precedes && c.n !== canvasN && same({ ...after, hint: 0 }, { ...before, hint: 0 }) && !after.hint, JSON.stringify({ c, before, after }));

    // ---- 4. Tab and Shift+Tab on the editor's own controls move focus, toggle nothing.
    await page.focus('.tb-tool-btn >> nth=0'); before = await panels(page); const f0 = await info(page);
    await page.keyboard.press('Tab'); const f1 = await info(page);
    await page.keyboard.press('Shift+Tab'); const f2 = await info(page); after = await panels(page);
    rec(`${T}: Tab on a toolbar button moves focus, Shift+Tab moves back, panels untouched`, f1.n !== f0.n && f2.n === f0.n && same(before, after), JSON.stringify({ f0, f1, f2 }));

    // ---- 5. FrozenTear's rule (2026-11-17): the other shortcuts act with the
    // canvas focused and do nothing from the page body or main.
    for (const where of ['page body', 'main', 'canvas']) {
      if (where === 'page body') await page.evaluate(() => document.activeElement && document.activeElement.blur());
      if (where === 'main') await page.focus('#main');
      if (where === 'canvas') { pt = await canvasPoint(page); await page.mouse.click(pt.x, pt.y); await page.waitForTimeout(100); }
      const acts = where === 'canvas';
      const verb = acts ? 'toggles' : 'does not toggle';
      before = await panels(page);
      await page.keyboard.press(']'); await page.waitForTimeout(120); let s1 = await panels(page);
      await page.keyboard.press(']'); await page.waitForTimeout(120); let s2 = await panels(page);
      rec(`${T}: ] ${verb} the right panel (focus on ${where})`, acts ? s1.right === !before.right && s2.right === before.right && s1.left === before.left : same(s1, before) && same(s2, before), JSON.stringify({ before, s1 }));
      await page.keyboard.press('['); await page.waitForTimeout(120); s1 = await panels(page);
      await page.keyboard.press('['); await page.waitForTimeout(120); s2 = await panels(page);
      rec(`${T}: [ ${verb} the bottom panel (focus on ${where})`, acts ? s1.bottom === !before.bottom && s2.bottom === before.bottom : same(s1, before) && same(s2, before), JSON.stringify({ before, s1 }));
      await page.click('.tb-tool-btn >> nth=0'); if (where === 'page body') await page.evaluate(() => document.activeElement.blur()); if (where === 'main') await page.focus('#main'); if (acts) await page.focus('.map-canvas-overlay');
      await page.keyboard.press('h'); await page.waitForTimeout(120); s1 = await panels(page);
      await page.keyboard.press('v'); await page.waitForTimeout(120); s2 = await panels(page);
      rec(`${T}: h and v ${acts ? 'pick' : 'do not pick'} the pan and select tools (focus on ${where})`, acts ? s1.tool === 1 && s2.tool === 0 : s1.tool === 0 && s2.tool === 0, `${s1.tool} ${s2.tool}`);
    }
    const titles = await page.evaluate(() => [...document.querySelectorAll('.sidebar-collapse-btn')].map(b => b.title));
    rec(`${T}: panel button help text matches the keys`, same(titles, ['Toggle panel (Tab while the map has focus)', 'Toggle panel (] while the map has focus)']), JSON.stringify(titles));
    const label = await page.getAttribute('.map-canvas-overlay', 'aria-label');
    rec(`${T}: the canvas names its keys`, !!label && label.includes('Esc or Shift+Tab leaves the map') && (await page.getAttribute('.map-canvas-overlay', 'tabindex')) === '-1', String(label));
    rec(`${T}: no page error`, errors.length === 0, errors.join(' | '));
    await ctx.close();
  }
}
await browser.close();
finish();
