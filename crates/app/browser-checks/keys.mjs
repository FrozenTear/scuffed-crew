// Keyboard checks for the S2 site shell and the strategy editor (PR 693).
//
// 1. Shell: skip link, group panels, account menu, phone drawer (from S2).
// 2. Vision nits 6 and 7: resize across the phone breakpoint; a trial recruit
//    keeps Apply.
// 3. Vision blocking 1: no editor shortcut fires on a focused button or link;
//    the same shortcuts work in the map canvas. Esc in the canvas cancels a
//    drawing or selection first, then leaves (notes 5 and 10).
// 4. Vision blocking 2: the map picker is a modal dialog.
// 5. Vision r4 nits: 3 (ring after a key), 13 (drawer), 14 (Ctrl+S in the
//    picker), 15 (Cmd chords), 16 (selects and sliders), 17 (stored map id
//    not the picker's), 20 (no hint on mouse focus), 21 (aria-current).
// 6. Vision r5 nits: 22 (Ctrl/Cmd+S behind the drawer), 23 (Tab from outside
//    the picker), 24 (no Cmd+Y), 25 (arrows on a select), and Tab out of a
//    group panel closes it.
// 7. FrozenTear's rule (2026-11-17): on /strategy/editor every shortcut,
//    Ctrl/Cmd chords included, acts only with focus in the map canvas. Not on
//    buttons, links, the slider, the select, the page body or main.
// 8. QA nit 1 on 8ec68607: the current page's own nav link has
//    aria-current="page" in the live DOM, and a nested page marks its parent
//    "true".
import { launch, makeContext, open, readState, reporter } from './lib.mjs';

const { rec, finish } = reporter('keys');
const STRATEGIES = readState('strategy.json');
const EDITOR = '/strategy/editor/' + STRATEGIES.id;
const active = page => page.evaluate(() => { const a = document.activeElement; return a ? { id: a.id, tag: a.tagName, cls: a.className, text: (a.textContent || '').trim().slice(0, 30), inDrawer: !!a.closest('#site-drawer') } : null; });
const browser = await launch();

// ---------------------------------------------------------------- 1. shell
for (const role of ['guest', 'member', 'admin']) {
  // desktop
  let ctx = await makeContext(browser, role, 1440); let page = await ctx.newPage();
  for (const path of ['/', '/admin', '/strategy']) {
    await open(page, path);
    await page.keyboard.press('Tab');
    let a = await active(page);
    const vis = await page.evaluate(() => { const r = document.activeElement.getBoundingClientRect(); return r.top >= 0 && r.left >= 0 && r.bottom <= innerHeight; });
    rec(`${role} ${path}: skip link is the first Tab stop and is visible`, a.cls === 'skip-link' && vis, JSON.stringify(a));
    await page.keyboard.press('Enter'); await page.waitForTimeout(100);
    a = await active(page);
    rec(`${role} ${path}: skip link lands on #main`, a.id === 'main' && a.tag === 'MAIN', JSON.stringify(a));
    await page.keyboard.press('Tab'); a = await active(page);
    const inMain = await page.evaluate(() => !!document.activeElement.closest('#main') || !!document.activeElement.closest('.site-footer'));
    rec(`${role} ${path}: next Tab after skip goes into the page, not back to the bar`, inMain, JSON.stringify(a));
  }
  await open(page, '/');
  const toggles = await page.$$eval('.d1-group-btn', bs => bs.map(b => b.id));
  if (role !== 'guest') toggles.push('nav-account-toggle');
  for (const id of toggles) {
    const sel = '#' + id; const panel = await page.getAttribute(sel, 'aria-controls');
    const shown = () => page.evaluate(p => { const e = document.getElementById(p); return !!e && getComputedStyle(e).display !== 'none'; }, panel);
    rec(`${role} ${id}: aria-controls names a real panel, closed at start`, !!panel && !(await shown()) && (await page.getAttribute(sel, 'aria-expanded')) === 'false');
    await page.focus(sel); await page.keyboard.press('Enter'); await page.waitForTimeout(150);
    rec(`${role} ${id}: Enter opens`, (await page.getAttribute(sel, 'aria-expanded')) === 'true' && await shown());
    await page.keyboard.press('Tab'); let a = await active(page);
    const inPanel = await page.evaluate(p => !!document.activeElement.closest('#' + p), panel);
    rec(`${role} ${id}: Tab moves into the panel`, inPanel, JSON.stringify(a));
    await page.keyboard.press('Escape'); await page.waitForTimeout(200);
    a = await active(page);
    rec(`${role} ${id}: Escape closes and focus returns to the button`, (await page.getAttribute(sel, 'aria-expanded')) === 'false' && !(await shown()) && a.id === id, JSON.stringify(a));
    // space key
    await page.keyboard.press('Space'); await page.waitForTimeout(150);
    rec(`${role} ${id}: Space opens`, (await page.getAttribute(sel, 'aria-expanded')) === 'true');
    await page.keyboard.press('Space'); await page.waitForTimeout(150);
    rec(`${role} ${id}: Space again closes`, (await page.getAttribute(sel, 'aria-expanded')) === 'false');
    // click outside
    await page.click(sel); await page.waitForTimeout(150);
    const url = page.url();
    await page.mouse.click(720, 700); await page.waitForTimeout(200);
    a = await active(page);
    rec(`${role} ${id}: click outside closes, no navigation`, (await page.getAttribute(sel, 'aria-expanded')) === 'false' && page.url() === url, 'focus=' + JSON.stringify(a));
    rec(`${role} ${id}: dismiss layer is gone after close`, (await page.$('.nav-dismiss-layer')) === null);
    // click on the bar itself
    await page.click(sel); await page.waitForTimeout(150);
    await page.mouse.click(900, 28); await page.waitForTimeout(200);
    rec(`${role} ${id}: click on empty top bar closes`, (await page.getAttribute(sel, 'aria-expanded')) === 'false');
    // Tab past the last item: focus leaves the panel and the panel closes.
    await page.focus(sel); await page.keyboard.press('Enter'); await page.waitForTimeout(150);
    const items = await page.evaluate(p => [...document.querySelectorAll(`#${p} a[href], #${p} button:not([disabled])`)].filter(e => e.getBoundingClientRect().width > 0).length, panel);
    for (let i = 0; i < items; i++) await page.keyboard.press('Tab');
    const lastIn = await page.evaluate(p => !!document.activeElement.closest('#' + p), panel);
    await page.keyboard.press('Tab'); await page.waitForTimeout(200); a = await active(page);
    const outside = await page.evaluate(p => !document.activeElement.closest('#' + p), panel);
    rec(`${role} ${id}: Tab past the last of ${items} items leaves the panel and closes it, no dismiss layer`, items > 0 && lastIn && outside && (await page.getAttribute(sel, 'aria-expanded')) === 'false' && !(await shown()) && (await page.$('.nav-dismiss-layer')) === null, JSON.stringify(a));
  }
  // only one open at a time
  await page.click('#' + toggles[0]); await page.click('#' + toggles[1]); await page.waitForTimeout(150);
  rec(`${role}: opening a second group closes the first`, (await page.getAttribute('#' + toggles[0], 'aria-expanded')) === 'false' && (await page.getAttribute('#' + toggles[1], 'aria-expanded')) === 'true');
  // follow a link in the panel
  const panel2 = await page.getAttribute('#' + toggles[1], 'aria-controls');
  await page.click(`#${panel2} a >> nth=0`); await page.waitForTimeout(800);
  rec(`${role}: a followed panel link navigates and closes the panel`, new URL(page.url()).pathname !== '/' && (await page.getAttribute('#' + toggles[1], 'aria-expanded')) === 'false', page.url());
  await ctx.close();

  // phone
  ctx = await makeContext(browser, role, 390); page = await ctx.newPage();
  for (const path of ['/', role === 'admin' ? '/admin/applications' : '/members']) {
    await open(page, path);
    await page.keyboard.press('Tab'); let a = await active(page);
    rec(`${role} phone ${path}: skip link first`, a.cls === 'skip-link', JSON.stringify(a));
    await page.keyboard.press('Enter'); await page.waitForTimeout(100); a = await active(page);
    rec(`${role} phone ${path}: skip link lands on #main`, a.id === 'main');
    await page.focus('#site-drawer-toggle'); await page.keyboard.press('Enter'); await page.waitForTimeout(300);
    a = await active(page);
    rec(`${role} phone ${path}: Menu opens the drawer, focus on Close`, (await page.getAttribute('#site-drawer-toggle', 'aria-expanded')) === 'true' && a.id === 'site-drawer-close', JSON.stringify(a));
    const inert = await page.evaluate(() => ['.d1-top', '.site-main', '.site-footer', '.site-tab-bar', '.site-subnav'].map(s => { const e = document.querySelector(s); return e ? s + '=' + e.inert + '/' + e.getAttribute('inert') : null; }).filter(Boolean));
    rec(`${role} phone ${path}: background is inert while the drawer is open`, inert.every(s => s.includes('=true/')), inert.join(' '));
    const cover = await page.evaluate(() => [[195, 30], [195, 400], [195, 830], [5, 5], [385, 840]].every(([x, y]) => { const e = document.elementFromPoint(x, y); return e && e.closest('#site-drawer'); }));
    rec(`${role} phone ${path}: drawer covers the whole viewport`, cover);
    const bgc = await page.evaluate(() => getComputedStyle(document.getElementById('site-drawer')).backgroundColor);
    rec(`${role} phone ${path}: drawer fill is solid`, !/rgba\(.*,\s*0?\.\d+\)|transparent/.test(bgc), bgc);
    const stops = await page.evaluate(() => [...document.querySelectorAll('#site-drawer a[href], #site-drawer button:not([disabled])')].filter(e => e.getBoundingClientRect().width > 0).length);
    let leaked = null; const seen = new Set();
    for (let i = 0; i < stops + 3; i++) { await page.keyboard.press('Tab'); a = await active(page); if (!a.inDrawer) { leaked = a; break; } seen.add(await page.evaluate(() => [...document.querySelectorAll('#site-drawer a[href], #site-drawer button')].indexOf(document.activeElement))); }
    rec(`${role} phone ${path}: Tab stays in the drawer over ${stops + 3} presses (${stops} stops, ${seen.size} seen)`, !leaked && seen.size === stops, leaked ? JSON.stringify(leaked) : '');
    a = await active(page);
    await page.focus('#site-drawer-close'); await page.keyboard.press('Shift+Tab'); a = await active(page);
    rec(`${role} phone ${path}: Shift+Tab from the first stop wraps to the last`, a.inDrawer && a.id !== 'site-drawer-close', JSON.stringify(a));
    await page.keyboard.press('Tab'); a = await active(page);
    rec(`${role} phone ${path}: Tab from the last stop wraps to Close`, a.id === 'site-drawer-close', JSON.stringify(a));
    await page.keyboard.press('Escape'); await page.waitForTimeout(300); a = await active(page);
    const after = await page.evaluate(() => ['.d1-top', '.site-main', '.site-tab-bar'].map(s => document.querySelector(s).getAttribute('inert')));
    rec(`${role} phone ${path}: Escape closes the drawer, focus returns to the Menu tab`, (await page.getAttribute('#site-drawer-toggle', 'aria-expanded')) === 'false' && a.id === 'site-drawer-toggle', JSON.stringify(a));
    rec(`${role} phone ${path}: inert is removed after close`, after.every(v => v === null), JSON.stringify(after));
    // close button
    await page.click('#site-drawer-toggle'); await page.waitForTimeout(250); await page.click('#site-drawer-close'); await page.waitForTimeout(300); a = await active(page);
    rec(`${role} phone ${path}: Close button closes, focus returns to the Menu tab`, a.id === 'site-drawer-toggle' && !(await page.evaluate(() => document.getElementById('site-drawer').classList.contains('is-open'))), JSON.stringify(a));
    // link in drawer
    await page.click('#site-drawer-toggle'); await page.waitForTimeout(250);
    await page.click('#site-drawer a:has-text("Forum")'); await page.waitForTimeout(900);
    rec(`${role} phone ${path}: a drawer link navigates and closes the drawer`, new URL(page.url()).pathname === '/forum' && !(await page.evaluate(() => document.getElementById('site-drawer').classList.contains('is-open'))) && (await page.evaluate(() => document.querySelector('.site-main').getAttribute('inert'))) === null, page.url());
  }
  if (role !== 'guest') {
    await open(page, '/');
    await page.focus('#nav-account-toggle'); await page.keyboard.press('Enter'); await page.waitForTimeout(200);
    const box = await page.evaluate(() => { const r = document.getElementById('nav-account-panel').getBoundingClientRect(); return [r.left, r.right, r.top, r.bottom, innerWidth]; });
    rec(`${role} phone: account menu opens inside the viewport`, box[0] >= 0 && box[1] <= box[4] && box[1] - box[0] > 100, box.join(','));
    await page.keyboard.press('Escape'); await page.waitForTimeout(200); const a = await active(page);
    rec(`${role} phone: Escape closes the account menu, focus returns`, a.id === 'nav-account-toggle');
  }
  await ctx.close();
}

// ------------------------------------------- 2. resize (nit 6), recruit (nit 7)
{
  const ctx = await makeContext(browser, 'member', 1440); const page = await ctx.newPage();
  await open(page, '/');
  await page.click('#nav-group-community-toggle'); await page.waitForTimeout(150);
  const wasOpen = (await page.getAttribute('#nav-group-community-toggle', 'aria-expanded')) === 'true';
  await page.setViewportSize({ width: 390, height: 844 }); await page.waitForTimeout(400);
  let a = await active(page);
  rec('nit 6: a desktop group panel closes on entering the phone layout, no dismiss layer, focus on #main', wasOpen && (await page.getAttribute('#nav-group-community-toggle', 'aria-expanded')) === 'false' && (await page.$('.nav-dismiss-layer')) === null && a.id === 'main', JSON.stringify(a));
  await page.setViewportSize({ width: 1440, height: 900 }); await page.waitForTimeout(300);
  await page.click('#nav-account-toggle'); await page.waitForTimeout(150);
  await page.setViewportSize({ width: 390, height: 844 }); await page.waitForTimeout(400);
  a = await active(page);
  rec('nit 6: the account menu closes on entering the phone layout, focus on #main', (await page.getAttribute('#nav-account-toggle', 'aria-expanded')) === 'false' && (await page.$('.nav-dismiss-layer')) === null && a.id === 'main', JSON.stringify(a));
  await page.click('#site-drawer-toggle'); await page.waitForTimeout(300);
  const drawerWas = await page.evaluate(() => document.getElementById('site-drawer').classList.contains('is-open'));
  await page.setViewportSize({ width: 1440, height: 900 }); await page.waitForTimeout(400);
  a = await active(page);
  rec('nit 6: the drawer closes past 820, nothing inert, focus on #main (not the body)', drawerWas && !(await page.evaluate(() => document.getElementById('site-drawer').classList.contains('is-open'))) && (await page.evaluate(() => document.querySelectorAll('[inert]').length)) === 0 && a.id === 'main', JSON.stringify(a));
  await ctx.close();
}
for (const width of [1440, 390]) {
  const ctx = await makeContext(browser, 'recruit', width); const page = await ctx.newPage(); await open(page, '/');
  await page.click('#nav-account-toggle'); await page.waitForTimeout(200);
  const vis = await page.evaluate(() => { const a = document.querySelector('#nav-account-panel a[href="/apply"]'); return !!a && a.getBoundingClientRect().height > 0; });
  rec(`nit 7, ${width}: a trial recruit (Recruit01) has Apply in the account menu`, vis);
  if (vis) { await page.click('#nav-account-panel a[href="/apply"]'); await page.waitForTimeout(900); }
  rec(`nit 7, ${width}: it opens /apply`, new URL(page.url()).pathname === '/apply', page.url());
  await ctx.close();
}

// ------------------------------------------------- 3. editor shortcuts (blocking 1)
// Every single-key binding (34). Space is checked on its own: on a button it
// must press the button and nothing else.
const SINGLE_KEYS = ['v', '1', 'h', '2', 'm', '3', 'r', '4', 'a', '5', 'w', '6', 't', '7', 'e', '8', ']', '[', '\\', 'Delete', 'Backspace', '+', '=', '-', '_', '0', 'k', 'l', 'ArrowRight', 'j', 'ArrowLeft', 'Home', 'End'];
const state = page => page.evaluate(() => {
  const st = document.querySelector('.editor-statusbar').textContent;
  const closed = sel => document.querySelector(sel).classList.contains('collapsed');
  return {
    left: !closed('.editor-sidebar-left'), right: !closed('.editor-sidebar-right'), bottom: !closed('.element-list-wrapper'),
    tool: [...document.querySelectorAll('.tb-tool-btn')].findIndex(b => b.classList.contains('active')),
    phase: (st.match(/Phase: (\d+\/\d+)/) || [])[1], elements: (st.match(/Elements: (\d+)/) || [])[1],
    playing: !!document.querySelector('.tl-ctrl-btn.playing'), zoom: document.querySelector('.tb-zoom-display')?.textContent,
    hp: !!document.querySelector('.tb-hp-toggle.active'), path: location.pathname,
  };
});
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const canvasPoint = page => page.evaluate(() => {
  const c = document.querySelector('.map-canvas-overlay'); const r = c.getBoundingClientRect();
  for (let fy = 0.5; fy > 0.05; fy -= 0.05) for (let fx = 0.5; fx < 0.96; fx += 0.05) {
    const x = Math.round(r.left + r.width * fx), y = Math.round(r.top + r.height * fy);
    if (document.elementFromPoint(x, y) === c) return { x, y };
  }
  return null;
});
const focusedIs = (page, sel) => page.evaluate(s => document.activeElement === document.querySelector(s), sel);
// down/up sends the bare key: `press('+')` would parse '+' as a combo.
const press1 = async (page, k) => { const n = k === '\\' ? 'Backslash' : k; await page.keyboard.down(n); await page.keyboard.up(n); };

for (const role of ['guest', 'member']) {
  for (const width of [1440, 390]) {
    const T = `editor ${role} ${width}`;
    const ctx = await makeContext(browser, role, width); const page = await ctx.newPage();
    const errors = []; page.on('pageerror', e => errors.push(String(e).slice(0, 120)));
    page.on('dialog', d => d.dismiss());
    await open(page, EDITOR);
    // nit 4: the panel buttons are named by what they do.
    const names = await page.evaluate(() => [...document.querySelectorAll('.sidebar-collapse-btn, .panel-collapse-bottom')].map(b => [b.getAttribute('aria-label'), b.getAttribute('aria-expanded')]));
    const s0 = await state(page);
    rec(`${T}: nit 4, panel buttons have real names and aria-expanded`, same(names, [[s0.left ? 'Hide team panel' : 'Show team panel', String(s0.left)], [s0.bottom ? 'Hide elements panel' : 'Show elements panel', String(s0.bottom)], [s0.right ? 'Hide properties panel' : 'Show properties panel', String(s0.right)]]), JSON.stringify(names));

    // Three phases, the middle one selected, so arrows, Home and End would show.
    for (const name of ['Setup', 'Push', 'Hold']) { await page.fill('.add-phase-form input', name); await page.click('.add-phase-btn'); await page.waitForTimeout(80); }
    await page.evaluate(() => document.querySelector('.tl-ctrl-btn[title="Last Phase"]').click()); await page.waitForTimeout(80);
    await page.evaluate(() => document.querySelector('.tl-ctrl-btn[title="Previous Phase"]').click()); await page.waitForTimeout(80);
    // Known S2 gap: at 390 with both side panels open the canvas is 0px wide.
    // Close them with their buttons (the timeline above lives in the right one).
    if (width === 390) { for (const side of ['left', 'right']) await page.click(`.editor-sidebar-${side} .sidebar-collapse-btn`); await page.waitForTimeout(200); }
    // One marker, selected, so Delete and Backspace would show.
    let pt = await canvasPoint(page);
    await page.click('.tb-tool-btn[title="Player Marker (M)"]'); await page.mouse.click(pt.x, pt.y); await page.waitForTimeout(100);
    await page.click('.tb-tool-btn[title="Select (V)"]'); await page.mouse.click(pt.x, pt.y); await page.waitForTimeout(100);
    // The hp toggle and zoom start away from their defaults so every key shows.
    const base = await state(page);
    // The seeded strategy may already have phases: work from the total.
    const N = Number((base.phase || '0/0').split('/')[1]);
    const P = i => `${i}/${N}`;
    rec(`${T}: setup: ${N} phases (3 added) on the one before last, one element`, N >= 3 && base.phase === P(N - 1) && base.elements === '1', JSON.stringify(base));

    // Each single-key shortcut on a focused button and on a focused link.
    const link = await page.evaluate(() => { const a = [...document.querySelectorAll('.d1-top a[href]')].find(e => e.getBoundingClientRect().width > 0); a.id = a.id || 'probe-link'; return '#' + a.id; });
    for (const [what, sel] of [['button', '.tb-tool-btn[title="Zoom In"]'], ['link', link]]) {
      const fired = [];
      for (const key of SINGLE_KEYS) {
        // v and 1 select the Select tool: start from Pan so a fired one shows.
        const tool = key === 'v' || key === '1' ? 1 : 0;
        await page.evaluate(i => document.querySelectorAll('.tb-tool-btn')[i].click(), tool); await page.waitForTimeout(30);
        await page.focus(sel); const before = await state(page);
        await press1(page, key); await page.waitForTimeout(60);
        const after = await state(page);
        if (!same(before, after) || !(await focusedIs(page, sel))) fired.push(key + ' ' + JSON.stringify(after));
      }
      rec(`${T}: none of the ${SINGLE_KEYS.length} single-key shortcuts (Space is checked next) fires on a focused ${what}`, fired.length === 0, fired.join(' | '));
    }
    // Space on a focused button only presses that button.
    await page.focus('.tb-tool-btn[title="Zoom In"]'); let before = await state(page);
    await page.keyboard.press('Space'); await page.waitForTimeout(100); let after = await state(page); let again;
    rec(`${T}: Space on a focused button presses it (zoom ${before.zoom} to ${after.zoom}) and does not toggle playback`, after.zoom !== before.zoom && same({ ...after, zoom: 0 }, { ...before, zoom: 0 }), JSON.stringify({ before, after }));
    await page.focus(link); before = await state(page);
    await page.keyboard.press('Space'); await page.waitForTimeout(100); after = await state(page);
    rec(`${T}: Space on a focused link does not toggle playback or navigate`, same(before, after), JSON.stringify({ before, after }));
    // FrozenTear's rule: the Ctrl/Cmd chords do nothing on a control either.
    // Undo has something to undo here (the marker), so a fired one shows.
    // Ctrl/Cmd+S is left to the browser (not prevented) and saves nothing.
    const CHORDS = ['Control+z', 'Control+Shift+z', 'Control+y', 'Meta+z', 'Meta+Shift+z', 'Control+s', 'Meta+s'];
    await page.evaluate(() => { window.__saveSeen = []; window.addEventListener('keydown', e => { if (e.key.toLowerCase() === 's' && (e.ctrlKey || e.metaKey)) window.__saveSeen.push(e.defaultPrevented); }); });
    const chordsDoNothing = async (what, focus, isFocused) => {
      await page.evaluate(() => { window.__saveSeen = []; });
      await focus(); before = await state(page); const url0 = page.url();
      for (const c of CHORDS) await page.keyboard.press(c);
      await page.waitForTimeout(300); after = await state(page);
      const seen = await page.evaluate(() => window.__saveSeen);
      // Focus stays put, or the browser's own Ctrl+Z (its editing history:
      // the phase name typed above) moved it to that text field. That is the
      // browser acting, not the editor.
      const kept = await isFocused() || await page.evaluate(() => document.activeElement.matches('input:not([type]), input[type=text], textarea'));
      const focusNow = await active(page);
      rec(`${T}: none of the ${CHORDS.length} editor chords (undo, redo, save, Ctrl and Cmd) acts with focus on the ${what}; Ctrl/Cmd+S left to the browser`, same(before, after) && page.url() === url0 && same(seen, [false, false]) && kept, JSON.stringify({ before, after, seen, kept, focusNow }));
    };
    for (const [what, sel] of [['button', '.tb-tool-btn[title="Zoom In"]'], ['link', link], ['opacity slider', '.tb-opacity input[type=range]'], ['visibility select', '.tb-visibility-select']]) {
      if (!(await page.$(sel))) { rec(`${T}: ${what} is on the page`, false); continue; }
      await chordsDoNothing(what, () => page.focus(sel), () => focusedIs(page, sel));
    }
    // The page body and main: no single key and no chord acts.
    const blurAll = () => page.evaluate(() => document.activeElement && document.activeElement.blur());
    for (const [what, focus, isFocused] of [
      ['page body', blurAll, () => page.evaluate(() => document.activeElement === document.body)],
      ['main landmark', () => page.focus('#main'), () => focusedIs(page, '#main')],
    ]) {
      const fired = [];
      for (const key of [...SINGLE_KEYS, ' ']) {
        const tool = key === 'v' || key === '1' ? 1 : 0;
        await page.evaluate(i => document.querySelectorAll('.tb-tool-btn')[i].click(), tool); await page.waitForTimeout(30);
        await focus(); const b = await state(page);
        await press1(page, key); await page.waitForTimeout(60);
        const s = await state(page);
        if (!same(b, s) || !(await isFocused())) fired.push(JSON.stringify(key) + ' ' + JSON.stringify(s));
      }
      rec(`${T}: none of the ${SINGLE_KEYS.length + 1} single-key shortcuts (Space included) acts with focus on the ${what}`, fired.length === 0, fired.join(' | '));
      await chordsDoNothing(what, focus, isFocused);
    }
    // Nit 16 and 25: a slider and a select keep their own keys.
    for (const [what, sel, key] of [['opacity slider', '.tb-opacity input[type=range]', 'ArrowRight'], ['visibility select', '.tb-visibility-select', 'ArrowDown']]) {
      if (!(await page.$(sel))) continue;
      if (what === 'opacity slider') {
        // The slider keeps its arrows: the value moves, the phase does not.
        await page.focus(sel); before = await state(page); const v0 = await page.$eval(sel, e => e.value);
        await page.keyboard.press(key); await page.waitForTimeout(100); after = await state(page); const v1 = await page.$eval(sel, e => e.value);
        rec(`${T}: nit 16, ArrowRight on the slider moves the slider (${v0} to ${v1}), not the phase`, v1 !== v0 && after.phase === before.phase, JSON.stringify({ before, after }));
      } else {
        // Nit 25: the select keeps its arrows too. ArrowRight and ArrowLeft
        // (the phase keys) leave the phase alone; ArrowDown is the select's.
        const moved = []; const values = [await page.$eval(sel, e => e.value)];
        for (const k of ['ArrowRight', 'ArrowLeft', key]) {
          await page.focus(sel); before = await state(page);
          await page.keyboard.press(k); await page.waitForTimeout(100); after = await state(page);
          values.push(await page.$eval(sel, e => e.value));
          if (after.phase !== before.phase || !(await focusedIs(page, sel))) moved.push(k + ' ' + JSON.stringify({ before, after }));
        }
        rec(`${T}: nit 25, ArrowRight, ArrowLeft and ${key} on the select do not move the phase, focus stays, ${key} moves the select (${values.join(' > ')})`, moved.length === 0 && values[3] !== values[2], moved.join(' | ') + ' ' + values.join(' > '));
      }
    }
    // Saving moved the strategy off this tab's URL? Keep the same page either way.
    if (new URL(page.url()).pathname !== EDITOR) { await open(page, EDITOR); }

    // The same shortcuts are active in the canvas. Focus by clicking an empty spot.
    const empty = await page.evaluate(([x, y]) => {
      const c = document.querySelector('.map-canvas-overlay'); const r = c.getBoundingClientRect();
      for (const [fx, fy] of [[0.15, 0.15], [0.85, 0.15], [0.15, 0.85], [0.85, 0.85]]) { const px = Math.round(r.left + r.width * fx), py = Math.round(r.top + r.height * fy); if (document.elementFromPoint(px, py) === c && Math.hypot(px - x, py - y) > 80) return { x: px, y: py }; }
      return null;
    }, [pt.x, pt.y]);
    await page.mouse.click(empty.x, empty.y); await page.waitForTimeout(100);
    rec(`${T}: canvas focused by a click`, await focusedIs(page, '.map-canvas-overlay'));
    // nit 3: the focus ring is drawn inside the canvas edge, not clipped.
    const ring = await page.evaluate(() => { const s = getComputedStyle(document.querySelector('.map-canvas-overlay')); return [s.outlineStyle, s.outlineOffset, s.outlineWidth]; });
    rec(`${T}: nit 3, the focused canvas shows an inset ring`, ring[0] === 'solid' && ring[1] === '-2px' && ring[2] === '2px', ring.join(' '));
    // nit 9: the live hint node is mounted before focus and only its text changes.
    const hintNow = () => page.evaluate(() => { const h = document.querySelector('.canvas-key-hint'); return h && [h.getAttribute('role'), h.textContent, h.getBoundingClientRect().height]; });
    let hint = await hintNow();
    rec(`${T}: nits 9 and 20, the status hint is mounted and empty after a mouse focus`, !!hint && hint[0] === 'status' && hint[1] === '', JSON.stringify(hint));
    const inCanvas = [
      [']', s => s.right !== base.right], [']', s => s.right === base.right],
      ['[', s => s.bottom !== base.bottom], ['[', s => s.bottom === base.bottom],
      ['h', s => s.tool === 1], ['v', s => s.tool === 0],
      [' ', s => s.playing], [' ', s => !s.playing],
      ['ArrowLeft', s => s.phase === P(N - 2)], ['ArrowRight', s => s.phase === P(N - 1)],
      ['End', s => s.phase === P(N)], ['Home', s => s.phase === P(1)],
      ['+', (s, b) => s.zoom !== b.zoom], ['0', s => s.zoom === '100%'], ['-', (s, b) => s.zoom !== b.zoom], ['0', s => s.zoom === '100%'],
      ['\\', (s, b) => s.hp !== b.hp], ['\\', (s, b) => s.hp !== b.hp],
    ];
    const missed = [];
    for (const [key, ok] of inCanvas) {
      const b = await state(page); await press1(page, key); await page.waitForTimeout(80); const s = await state(page);
      if (!ok(s, b) || !(await focusedIs(page, '.map-canvas-overlay'))) missed.push(key + ' ' + JSON.stringify(s));
    }
    rec(`${T}: ${inCanvas.length} shortcut presses all act with the canvas focused`, missed.length === 0, missed.join(' | '));
    hint = await hintNow();
    rec(`${T}: nits 19 and 20, a key in the map shows the hint with the cancel step`, hint[1] === 'Tab: team panel. Esc: cancel, then leave the map.' && hint[2] > 0, JSON.stringify(hint));
    // Nit 3: after a key press the canvas also matches :focus-visible, and
    // the ring must still be the inset one.
    const ringAfter = await page.evaluate(() => { const c = document.querySelector('.map-canvas-overlay'); const s = getComputedStyle(c); return [c.matches(':focus-visible'), s.outlineStyle, s.outlineOffset, s.outlineWidth]; });
    rec(`${T}: nit 3, after a key the canvas is :focus-visible and the ring is still inset`, ringAfter[0] === true && ringAfter[1] === 'solid' && ringAfter[2] === '-2px' && ringAfter[3] === '2px', ringAfter.join(' '));
    await page.mouse.click(pt.x, pt.y); await page.waitForTimeout(100); before = await state(page);
    await page.keyboard.press('Delete'); await page.waitForTimeout(100); after = await state(page);
    rec(`${T}: Delete in the canvas deletes the selected element`, before.elements === '1' && after.elements === '0', `${before.elements} ${after.elements}`);

    // Notes 5 and 10: Esc cancels a selection or drawing first, a second Esc leaves.
    await page.keyboard.press('m'); await page.mouse.click(pt.x, pt.y); await page.keyboard.press('v'); await page.mouse.click(pt.x, pt.y); await page.waitForTimeout(100);
    await page.keyboard.press('Escape'); await page.waitForTimeout(100);
    const stayed = await focusedIs(page, '.map-canvas-overlay');
    await page.keyboard.press('Delete'); await page.waitForTimeout(100); const kept = await state(page);
    rec(`${T}: Esc with an element selected deselects it and keeps focus in the canvas`, stayed && kept.elements === '1', JSON.stringify(kept));
    await page.keyboard.press('Escape'); await page.waitForTimeout(150); let a = await active(page);
    rec(`${T}: a second Esc leaves the canvas to #main`, a.id === 'main', JSON.stringify(a));
    for (const [key, tool] of [['w', 'arrow'], ['r', 'route']]) {
      await page.mouse.click(empty.x, empty.y); await page.keyboard.press(key); await page.waitForTimeout(60);
      before = await state(page);
      await page.mouse.move(pt.x, pt.y); await page.mouse.down(); await page.mouse.move(pt.x + 30, pt.y + 20, { steps: 3 });
      await page.keyboard.press('Escape'); await page.waitForTimeout(80);
      const mid = await focusedIs(page, '.map-canvas-overlay');
      await page.mouse.move(pt.x + 60, pt.y + 40, { steps: 3 }); await page.mouse.up(); await page.waitForTimeout(100);
      after = await state(page);
      rec(`${T}: Esc mid-${tool} cancels the drawing (no element added), focus stays in the canvas`, mid && after.elements === before.elements, `${before.elements} ${after.elements} mid=${mid}`);
      await page.keyboard.press('Escape'); await page.waitForTimeout(150); a = await active(page);
      rec(`${T}: then Esc leaves the canvas (${tool})`, a.id === 'main', JSON.stringify(a));
    }
    // The chords act with the canvas focused (nit 15: Cmd as on macOS).
    // Markers are on the map with undo history behind them: count from here.
    await page.focus('.map-canvas-overlay'); await page.keyboard.press('v'); await page.waitForTimeout(60);
    const n0 = Number((await state(page)).elements); const less = String(n0 - 1), more = String(n0);
    const steps = [];
    for (const c of ['Control+z', 'Control+Shift+z', 'Meta+z', 'Meta+Shift+z', 'Meta+z']) { await page.keyboard.press(c); await page.waitForTimeout(100); steps.push((await state(page)).elements); }
    rec(`${T}: Ctrl+Z, Ctrl+Shift+Z, Cmd+Z and Cmd+Shift+Z undo and redo with the canvas focused`, n0 >= 1 && same(steps, [less, more, less, more, less]) && (await focusedIs(page, '.map-canvas-overlay')), `${n0}: ` + JSON.stringify(steps));
    // Nit 24: Cmd+Y is not bound (History on macOS): no redo, default kept.
    await page.evaluate(() => { window.__cmdY = []; window.addEventListener('keydown', e => { if (e.key.toLowerCase() === 'y' && e.metaKey) window.__cmdY.push(e.defaultPrevented); }); });
    await page.keyboard.press('Meta+y'); await page.waitForTimeout(100); after = await state(page);
    const cmdY = await page.evaluate(() => window.__cmdY);
    await page.keyboard.press('Control+y'); await page.waitForTimeout(100); again = await state(page);
    rec(`${T}: nit 24, in the canvas Cmd+Y does not redo and is left to the browser; Ctrl+Y redoes`, after.elements === less && same(cmdY, [false]) && again.elements === more, `${after.elements} ${JSON.stringify(cmdY)} ${again.elements}`);
    await page.evaluate(() => { window.__saveSeen = []; });
    await page.focus('.map-canvas-overlay'); await page.keyboard.press('Control+s'); await page.waitForTimeout(300);
    if (await page.$('.map-canvas-overlay')) { await page.focus('.map-canvas-overlay'); await page.keyboard.press('Meta+s'); await page.waitForTimeout(300); }
    const saved = await page.evaluate(() => window.__saveSeen);
    rec(`${T}: Ctrl+S and Cmd+S with the canvas focused are the editor's (browser Save Page held back)`, same(saved, [true, true]), JSON.stringify(saved));
    rec(`${T}: no page error`, errors.length === 0, errors.join(' | '));
    await ctx.close();
  }
}

// ------------------------------------------------- 4. map picker modal (blocking 2)
for (const role of ['guest', 'member']) {
  for (const width of [1440, 390]) {
    const T = `picker ${role} ${width}`;
    const ctx = await makeContext(browser, role, width); const page = await ctx.newPage();
    const errors = []; page.on('pageerror', e => errors.push(String(e).slice(0, 120)));
    await open(page, '/strategy/editor');
    const dlg = await page.evaluate(() => { const d = document.getElementById('map-picker'); if (!d) return null; const t = document.getElementById(d.getAttribute('aria-labelledby')); return { role: d.getAttribute('role'), modal: d.getAttribute('aria-modal'), name: t && t.textContent.trim() }; });
    rec(`${T}: opens on a new strategy as role=dialog, aria-modal, named "Select a Map"`, !!dlg && dlg.role === 'dialog' && dlg.modal === 'true' && dlg.name === 'Select a Map', JSON.stringify(dlg));
    let a = await active(page);
    rec(`${T}: focus moves into the picker on open (Close, no map yet)`, a.id === 'map-picker-close', JSON.stringify(a));
    const inert = () => page.evaluate(() => Object.fromEntries(['.skip-link', '.d1-top', '.site-tab-bar', '.editor-container', '.editor-statusbar', '#main'].map(s => [s, document.querySelector(s)?.inert])));
    let ins = await inert(page);
    rec(`${T}: the page behind is inert (skip link, top bar, tab bar, editor, status bar), main is not`, ins['.skip-link'] && ins['.d1-top'] && ins['.site-tab-bar'] && ins['.editor-container'] && ins['.editor-statusbar'] && ins['#main'] === false, JSON.stringify(ins));
    const hits = await page.evaluate(() => [[innerWidth / 2, 20], [innerWidth / 2, innerHeight - 20], [8, innerHeight / 2]].map(([x, y]) => !!document.elementFromPoint(x, y)?.closest('.map-picker-overlay')));
    rec(`${T}: the overlay is on top of the top bar and the tab bar (z-index 110)`, hits.every(Boolean), JSON.stringify(hits));
    // No editor shortcut runs behind the open picker, even from the body.
    // The whole editor state (panels, tool, phase, playback, zoom), not only the panels.
    const before = await state(page);
    await page.evaluate(() => document.activeElement.blur());
    for (const key of [']', '[', 'h', 'Space', 'ArrowRight', 'End', '+', '\\', 'k']) await press1(page, key === 'Space' ? ' ' : key);
    await page.waitForTimeout(100);
    const afterKeys = await state(page);
    rec(`${T}: editor shortcuts do nothing behind the open picker`, same(before, afterKeys) && !!(await page.$('#map-picker')), JSON.stringify({ before, afterKeys }));
    // Nit 23 (QA nit 3 on 8ec68607): start on a real element outside the
    // picker. `main` is the one focusable element outside it that is not
    // inert while it is open; a blur would leave the browser's Tab start
    // inside the picker, so that version could not fail.
    const startOutside = await page.evaluate(() => { const m = document.getElementById('main'); m.focus(); return document.activeElement === m && !m.closest('#map-picker'); });
    await page.keyboard.press('Tab'); a = await page.evaluate(() => !!document.activeElement.closest('#map-picker'));
    rec(`${T}: nit 23, Tab with focus on #main (outside the picker) moves focus into the picker`, startOutside && a, `outside=${startOutside} in=${a}`);
    // Nit 14: Ctrl+S with the picker open does not reach the browser.
    await page.evaluate(() => { window.__saveBlocked = []; window.addEventListener('keydown', e => { if (e.key.toLowerCase() === 's' && (e.ctrlKey || e.metaKey)) window.__saveBlocked.push(e.defaultPrevented); }); });
    await page.focus('#map-picker-close');
    await page.keyboard.press('Control+s'); await page.keyboard.press('Meta+s'); await page.waitForTimeout(150);
    const blocked = await page.evaluate(() => window.__saveBlocked);
    rec(`${T}: nit 14, Ctrl+S and Cmd+S in the open picker are held back from the browser, picker stays open`, same(blocked, [true, true]) && !!(await page.$('#map-picker')), JSON.stringify(blocked));
    // Tab stays inside and visits every stop.
    const stops = await page.evaluate(() => [...document.querySelectorAll('#map-picker button:not([disabled])')].length);
    await page.focus('#map-picker-close'); const seen = new Set(); let leaked = null;
    for (let i = 0; i < stops + 3; i++) { await page.keyboard.press('Tab'); const r = await page.evaluate(() => ({ inside: !!document.activeElement.closest('#map-picker'), i: [...document.querySelectorAll('#map-picker button')].indexOf(document.activeElement) })); if (!r.inside) { leaked = await active(page); break; } seen.add(r.i); }
    rec(`${T}: Tab stays in the picker over ${stops + 3} presses and reaches all ${stops} stops`, !leaked && seen.size === stops, leaked ? JSON.stringify(leaked) : `${seen.size}`);
    await page.focus('#map-picker-close'); await page.keyboard.press('Shift+Tab');
    const last = await page.evaluate(() => { const all = [...document.querySelectorAll('#map-picker button')]; return document.activeElement === all[all.length - 1]; });
    await page.keyboard.press('Tab'); a = await active(page);
    rec(`${T}: Shift+Tab from Close wraps to the last map, Tab from there wraps to Close`, last && a.id === 'map-picker-close', JSON.stringify(a));
    // Esc closes, focus to the Map button, nothing inert.
    await page.keyboard.press('Escape'); await page.waitForTimeout(250); a = await active(page);
    ins = await inert(page);
    rec(`${T}: Esc closes the picker and focus returns to the Map button`, !(await page.$('#map-picker')) && a.id === 'editor-map-button', JSON.stringify(a));
    rec(`${T}: nothing is inert after closing`, Object.values(ins).every(v => v === false) && (await page.evaluate(() => document.querySelectorAll('[inert]').length)) === 0, JSON.stringify(ins));
    // Reopen with the keyboard, close with a click outside.
    await page.keyboard.press('Enter'); await page.waitForTimeout(250); a = await active(page);
    rec(`${T}: Enter on the Map button reopens it with focus inside`, !!(await page.$('#map-picker')) && a.id === 'map-picker-close', JSON.stringify(a));
    await page.mouse.click(4, Math.round((width === 390 ? 844 : 900) / 2)); await page.waitForTimeout(250); a = await active(page);
    rec(`${T}: a click outside closes it, focus back on the Map button`, !(await page.$('#map-picker')) && a.id === 'editor-map-button', JSON.stringify(a));
    // Space on a map button only picks the map.
    await page.keyboard.press('Enter'); await page.waitForTimeout(250); await page.keyboard.press('Tab');
    const playingBefore = !!(await page.$('.tl-ctrl-btn.playing'));
    await page.keyboard.press('Space'); await page.waitForTimeout(300); a = await active(page);
    const status = await page.evaluate(() => document.querySelector('.editor-statusbar').textContent);
    rec(`${T}: Space on a map in the picker picks it, closes the picker, toggles no playback`, !(await page.$('#map-picker')) && a.id === 'editor-map-button' && !/Map: None/.test(status) && !!(await page.$('.tl-ctrl-btn.playing')) === playingBefore, status);
    await page.keyboard.press('Enter'); await page.waitForTimeout(250); a = await active(page);
    const title = await page.evaluate(() => document.getElementById('map-picker-title')?.textContent.trim());
    const marks = await page.evaluate(() => ({ pressed: document.querySelectorAll('#map-picker [aria-pressed]').length, current: [...document.querySelectorAll('#map-picker [aria-current]')].map(e => e.id + '=' + e.getAttribute('aria-current')) }));
    rec(`${T}: reopened with a map, focus lands on the current map`, a.id === 'map-picker-selected' && title === 'Change Map', JSON.stringify(a));
    rec(`${T}: nit 21, only the current map has aria-current="true", no aria-pressed`, marks.pressed === 0 && same(marks.current, ['map-picker-selected=true']), JSON.stringify(marks));
    await page.keyboard.press('Escape'); await page.waitForTimeout(200);
    rec(`${T}: no page error`, errors.length === 0, errors.join(' | '));
    await ctx.close();
  }
}
// ----------------------------------------- 5. Vision r4: drawer (13), picker ids (17)
// Nit 13: with the phone drawer open the editor behind it takes no shortcut,
// even with focus on the body after a click on blank drawer space.
for (const role of ['guest', 'member']) {
  const T = `drawer ${role} 390`;
  const ctx = await makeContext(browser, role, 390); const page = await ctx.newPage();
  const errors = []; page.on('pageerror', e => errors.push(String(e).slice(0, 120)));
  await open(page, EDITOR);
  await page.click('#site-drawer-toggle'); await page.waitForTimeout(300);
  const blank = await page.evaluate(() => {
    const d = document.getElementById('site-drawer'); const r = d.getBoundingClientRect();
    for (let y = r.bottom - 10; y > r.top; y -= 10) for (const x of [r.left + 6, r.right - 6, (r.left + r.right) / 2]) {
      const e = document.elementFromPoint(x, y);
      if (e && e.closest('#site-drawer') && !e.closest('a[href], button, input, select, textarea, [tabindex]')) return { x: Math.round(x), y: Math.round(y) };
    }
    return null;
  });
  if (blank) { await page.mouse.click(blank.x, blank.y); await page.waitForTimeout(100); }
  const where = await page.evaluate(() => document.activeElement && document.activeElement.tagName);
  const mainInert = await page.evaluate(() => document.getElementById('main').inert);
  const before = await state(page);
  for (const key of [' ', 'Delete', 'ArrowRight', 'End', ']', '[', 'h', '+', '\\']) await press1(page, key);
  await page.waitForTimeout(150);
  const after = await state(page);
  rec(`${T}: nit 13, drawer open and focus on the page (${where}): no editor shortcut runs behind it`, !!blank && where !== null && mainInert && same(before, after), JSON.stringify({ blank, where, before, after }));
  // Nit 22: Ctrl+S and Cmd+S behind the drawer still hold back the browser's
  // Save Page dialog, and save nothing.
  await page.evaluate(() => { window.__saveBlocked = []; window.addEventListener('keydown', e => { if (e.key.toLowerCase() === 's' && (e.ctrlKey || e.metaKey)) window.__saveBlocked.push(e.defaultPrevented); }); });
  await page.keyboard.press('Control+s'); await page.keyboard.press('Meta+s'); await page.waitForTimeout(300);
  const blocked = await page.evaluate(() => window.__saveBlocked);
  const drawerStill = await page.evaluate(() => document.getElementById('site-drawer').classList.contains('is-open'));
  rec(`${T}: nit 22, Ctrl+S and Cmd+S with the drawer open are held back from the browser, nothing saved, drawer stays open`, same(blocked, [true, true]) && drawerStill && same(await state(page), after) && new URL(page.url()).pathname === EDITOR, JSON.stringify({ blocked, drawerStill, url: page.url() }));
  await page.keyboard.press('Escape'); await page.waitForTimeout(300);
  const closed = !(await page.evaluate(() => document.getElementById('site-drawer').classList.contains('is-open')));
  // Once it is closed the shortcuts work again, in the map canvas only.
  await page.evaluate(() => document.activeElement && document.activeElement.blur());
  const b1 = await state(page); await press1(page, ']'); await page.waitForTimeout(120); const a1 = await state(page);
  await page.focus('.map-canvas-overlay');
  const b2 = await state(page); await press1(page, ']'); await page.waitForTimeout(120); const a2 = await state(page);
  rec(`${T}: Esc closes the drawer; then ] does nothing from the body and acts in the map canvas`, closed && same(a1, b1) && a2.right !== b2.right, JSON.stringify({ b1, a1, b2, a2 }));
  rec(`${T}: no page error`, errors.length === 0, errors.join(' | '));
  await ctx.close();
}

// Nit 17, after issue 817: the map of a saved strategy is fixed (the update
// API takes no map), so the picker no longer opens there, whatever map id is
// stored (a display name such as "Dorado", or an id the picker does not
// list). The map item is text that says so, keeps its id with tabindex -1,
// and a click opens no picker. Picking a map is checked on a new strategy:
// the picked map is marked and focused when the picker opens again.
for (const width of [1440, 390]) {
  const ctx = await makeContext(browser, 'member', width); const page = await ctx.newPage();
  for (const [what, id] of [['stored by display name', STRATEGIES.byName], ['not in the picker', STRATEGIES.unknown]]) {
    const T = `picker ${width} map ${what}`;
    await open(page, '/strategy/editor/' + id);
    const m = await page.evaluate(() => { const e = document.getElementById('editor-map-button'); return e && { tag: e.tagName, tabindex: e.getAttribute('tabindex'), text: e.textContent.trim() }; });
    rec(`${T}: issue 817, the saved map is text "Map: ... (fixed once saved)" with tabindex -1`, !!m && m.tag === 'SPAN' && m.tabindex === '-1' && /^Map: .+ \(fixed once saved\)$/.test(m.text), JSON.stringify(m));
    await page.click('#editor-map-button'); await page.waitForTimeout(300);
    rec(`${T}: issue 817, a click on the fixed map opens no picker`, !(await page.$('#map-picker')), '');
  }
  {
    const T = `picker ${width} new strategy`;
    await open(page, '/strategy/editor');
    await page.click('.map-picker-item >> nth=1'); await page.waitForTimeout(400);
    const picked = await page.evaluate(() => document.querySelectorAll('.map-picker-item').length === 0 && document.getElementById('editor-map-button')?.textContent.trim());
    await page.click('#editor-map-button'); await page.waitForTimeout(300);
    const a = await active(page);
    const marks = await page.evaluate(() => [...document.querySelectorAll('#map-picker [aria-current]')].map(e => e.textContent.trim()));
    rec(`${T}: nit 17, the picked map is marked and focused when the picker opens again`, a && a.id === 'map-picker-selected' && marks.length === 1 && !!picked && picked.includes(marks[0]), JSON.stringify({ a, marks, picked }));
    await page.keyboard.press('Escape'); await page.waitForTimeout(200);
  }
  await ctx.close();
}

// ------------------------------------------- 8. aria-current (QA nit 1 on 8ec68607)
// Direct loads: every shell nav link to the current page says
// aria-current="page", no other link does. A nested page marks the link to
// its parent "true". Then one client-side move, so the router's update is
// checked too, not only the first render.
const NAV_LINKS = '.d1-top a[href], #site-drawer a[href], .site-subnav a[href], .admin-subnav a[href], .site-tab-bar a[href]';
const navMarks = page => page.evaluate(sel => [...document.querySelectorAll(sel)].map(a => ({
  path: new URL(a.href).pathname, cur: a.getAttribute('aria-current'),
  where: a.closest('#site-drawer') ? 'drawer' : a.closest('.site-tab-bar') ? 'tabbar' : a.closest('.admin-subnav') ? 'admin-subnav' : a.closest('.site-subnav') ? 'subnav' : 'top',
})), NAV_LINKS);
const pageMarked = (marks, path) => {
  const own = marks.filter(m => m.path === path);
  const wrong = marks.filter(m => m.path !== path && m.cur === 'page');
  return { ok: own.length > 0 && own.every(m => m.cur === 'page') && wrong.length === 0, detail: JSON.stringify({ own, wrong }) };
};
const SEED = readState('seed.json');
const M0 = readState('members.json').data[0].id;
for (const width of [1440, 390]) {
  for (const [role, paths, nested] of [
    ['guest', ['/', '/members', '/tournaments', '/strategy', '/patch-notes', '/strategy/heroes'], [['/forum/b/' + SEED.board.slug, '/forum']]],
    ['admin', ['/admin', '/admin/members'], [[`/admin/members/${M0}/matches`, '/admin/members']]],
  ]) {
    const ctx = await makeContext(browser, role, width); const page = await ctx.newPage();
    for (const p of paths) {
      await open(page, p);
      const r = pageMarked(await navMarks(page), p);
      const places = [...new Set((await navMarks(page)).filter(m => m.path === p).map(m => m.where))].join(', ');
      rec(`aria-current ${role} ${width} ${p}: every nav link to it (${places}) has aria-current="page", no other link does`, r.ok, r.detail);
    }
    for (const [p, parent] of nested) {
      await open(page, p);
      const marks = await navMarks(page);
      const own = marks.filter(m => m.path === parent);
      rec(`aria-current ${role} ${width} ${p}: the links to ${parent} have aria-current="true", none has "page"`, own.length > 0 && own.every(m => m.cur === 'true') && !marks.some(m => m.cur === 'page'), JSON.stringify(own));
    }
    await ctx.close();
  }
  // Client-side move: /members, then follow the Tournaments nav link.
  const ctx = await makeContext(browser, 'guest', width); const page = await ctx.newPage();
  await open(page, '/members');
  await page.evaluate(() => document.querySelector('.d1-top a[href="/tournaments"], .site-tab-bar a[href="/tournaments"]').click());
  await page.waitForFunction(() => location.pathname === '/tournaments', null, { timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(600);
  const marks = await navMarks(page);
  const r = pageMarked(marks, '/tournaments');
  const stale = marks.filter(m => m.path === '/members' && m.cur);
  rec(`aria-current guest ${width}: after a client-side move from /members to /tournaments, "page" moves with it`, new URL(page.url()).pathname === '/tournaments' && r.ok && stale.length === 0, r.detail + ' stale=' + JSON.stringify(stale));
  await ctx.close();
}

await browser.close();
finish();
