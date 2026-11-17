// Short keyboard pass. Results in $REPORT_DIR/report-keys.json.
//   node keys.mjs
import fs from 'node:fs';
import { B, launch, makeContext, open, REPORT_DIR } from './lib.mjs';
const results = [];
const check = (group, name, pass, detail = '') => { results.push({ group, name, pass: !!pass, detail: String(detail).slice(0, 300) }); console.log((pass ? 'PASS ' : 'FAIL ') + group + ': ' + name + (detail ? '  [' + String(detail).slice(0, 200) + ']' : '')); };
const browser = await launch();
const active = page => page.evaluate(() => { const a = document.activeElement; return a ? { id: a.id, tag: a.tagName, cls: a.className, text: (a.textContent || '').trim().slice(0, 40) } : null; });

for (const width of [1440, 390]) {
  const tag = ` (${width})`;
  // ---- Pager on /blog ------------------------------------------------------
  {
    const ctx = await makeContext(browser, 'guest', width, 'dark'); const page = await ctx.newPage();
    await open(page, '/blog');
    const state = () => page.evaluate(() => {
      const cur = document.querySelector('#blog-pager [aria-current="page"]'); const list = document.getElementById('blog-list');
      const top = document.querySelector('.d1-top'); const r = list ? list.getBoundingClientRect() : null;
      const tb = top ? top.getBoundingClientRect() : { bottom: 0 };
      const sticky = top && ['sticky', 'fixed'].includes(getComputedStyle(top).position);
      return { current: cur ? cur.textContent.trim() : null, activeId: document.activeElement && document.activeElement.id, listTop: r ? Math.round(r.top) : null, barBottom: Math.round(sticky ? tb.bottom : 0), scrollY: Math.round(scrollY), innerH: innerHeight, meta: (document.getElementById('blog-pager-meta') || {}).textContent, first: (document.querySelector('#blog-list .entry-row') || {}).textContent?.trim().slice(0, 40) };
    });
    const atTop = s => s.activeId === 'blog-list' && s.listTop >= s.barBottom - 1 && s.listTop < s.innerH * 0.6;
    const s0 = await state();
    check('pager', 'starts on page 1' + tag, s0.current === '1', JSON.stringify(s0));
    // Reach Next by Tab from the page-2 button, to prove it is in the tab order.
    await page.focus('#blog-pager-page-2'); await page.keyboard.press('Tab');
    const a1 = await active(page);
    check('pager', 'Tab from the last page button lands on Next' + tag, a1 && a1.id === 'blog-pager-next', JSON.stringify(a1));
    await page.keyboard.press('Enter'); await page.waitForTimeout(600);
    const s1 = await state();
    check('pager', 'Enter on Next goes to page 2' + tag, s1.current === '2' && s1.first !== s0.first, JSON.stringify(s1));
    check('pager', 'after Next, focus is on the list top and it is in view under the top bar' + tag, atTop(s1), JSON.stringify(s1));
    await page.focus('#blog-pager-next');
    const dis = await page.getAttribute('#blog-pager-next', 'aria-disabled');
    await page.keyboard.press('Enter'); await page.waitForTimeout(300);
    const s1b = await state();
    check('pager', 'Next on the last page is aria-disabled, stays focusable and does nothing' + tag, dis === 'true' && s1b.current === '2' && s1b.activeId === 'blog-pager-next', JSON.stringify({ dis, ...s1b }));
    await page.focus('#blog-pager-prev'); await page.keyboard.press('Space'); await page.waitForTimeout(600);
    const s2 = await state();
    check('pager', 'Space on Previous goes back to page 1' + tag, s2.current === '1', JSON.stringify(s2));
    check('pager', 'after Previous, focus is on the list top and it is in view' + tag, atTop(s2), JSON.stringify(s2));
    await page.focus('#blog-pager-page-2'); await page.keyboard.press('Enter'); await page.waitForTimeout(600);
    const s3 = await state();
    check('pager', 'Enter on the page 2 button goes to page 2' + tag, s3.current === '2', JSON.stringify(s3));
    check('pager', 'after the page button, focus is on the list top and it is in view' + tag, atTop(s3), JSON.stringify(s3));
    check('pager', 'range line (role=status) text changed with the page' + tag, s3.meta && s3.meta !== s0.meta, `${s0.meta} -> ${s3.meta}`);
    // The page is part of the address and of history.
    const url3 = await page.evaluate(() => location.pathname + location.search);
    check('pager', 'page 2 is in the address (?page=2)' + tag, url3 === '/blog?page=2', url3);
    const firstTitle = (await page.textContent('#blog-list .entry-row h2 a')).trim();
    await page.focus('#blog-list .entry-row h2 a'); await page.keyboard.press('Enter');
    await page.waitForFunction(() => location.pathname.startsWith('/blog/'), null, { timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(500);
    await page.goBack(); await page.waitForSelector('#blog-pager', { timeout: 8000 }).catch(() => {}); await page.waitForTimeout(800);
    const s4 = await state(); const url4 = await page.evaluate(() => location.pathname + location.search);
    check('pager', 'Back from a post opened on page 2 returns to page 2' + tag, s4.current === '2' && url4 === '/blog?page=2' && /^11 to 14/.test(s4.meta || ''), JSON.stringify({ url4, firstTitle, ...s4 }));
    await page.goBack(); await page.waitForTimeout(800);
    const s5 = await state();
    check('pager', 'Back again returns to page 1' + tag, s5.current === '1', JSON.stringify(s5));
    await page.goto(B + '/blog?page=2', { waitUntil: 'load' }); await page.waitForSelector('#blog-pager [aria-current="page"]', { timeout: 15000 }).catch(() => {}); await page.waitForTimeout(800);
    const s6 = await state(); const url6 = await page.evaluate(() => location.pathname + location.search);
    check('pager', 'opening /blog?page=2 directly shows page 2 and keeps the address' + tag, s6.current === '2' && url6 === '/blog?page=2', JSON.stringify({ url6, ...s6 }));
    await page.reload({ waitUntil: 'load' }); await page.waitForSelector('#blog-pager [aria-current="page"]', { timeout: 15000 }).catch(() => {}); await page.waitForTimeout(800);
    check('pager', 'reload keeps page 2' + tag, (await state()).current === '2');
    await page.evaluate(() => { const a = [...document.querySelectorAll('.site-footer a')].find(a => a.getAttribute('href') === '/blog'); a && a.click(); }); await page.waitForTimeout(800);
    const s7 = await state(); const url7 = await page.evaluate(() => location.pathname + location.search);
    check('pager', 'the Blog link from page 2 goes to page 1' + tag, s7.current === '1' && url7 === '/blog', JSON.stringify({ url7, ...s7 }));
    await ctx.close();
  }
  // ---- Tab titles ------------------------------------------------------------
  {
    const ctx = await makeContext(browser, 'guest', width, 'dark'); const page = await ctx.newPage();
    for (const [path, want] of [['/blog/scrim-review-three-weeks', 'Scrim review: what three weeks taught us | Blog | Test Crew'], ['/blog/no-such-post', 'Post not found | Blog | Test Crew'], ['/wiki/tank-basics', 'Tank basics | Wiki | Test Crew'], ['/wiki/no-such-topic', 'Page not found | Wiki | Test Crew'], ['/admin', 'No access | Test Crew'], ['/no-such-page', 'Page not found | Test Crew']]) {
      await open(page, path); await page.waitForTimeout(300);
      const t = await page.title();
      check('titles', `${path} is titled "${want}"` + tag, t === want, t);
    }
    // The page's own title does not stick after leaving it.
    await page.evaluate(() => { const a = [...document.querySelectorAll('.site-footer a')].find(a => a.getAttribute('href') === '/news'); a && a.click(); }); await page.waitForTimeout(600);
    const t2 = await page.title();
    check('titles', 'navigating on replaces the page title' + tag, t2 === 'News | Test Crew', t2);
    await ctx.close();
  }
  // ---- Withdraw confirm (Applicant01) ----------------------------------------
  {
    const ctx = await makeContext(browser, 'applicant', width, 'dark'); const page = await ctx.newPage();
    await open(page, '/apply');
    const trig = '#withdraw-application-trigger';
    if (await page.locator(trig).count()) {
      const reqs = []; page.on('request', r => { if (/withdraw/.test(r.url())) reqs.push(r.url()); });
      for (const [how, act] of [['Esc', async () => page.keyboard.press('Escape')], ['Enter on Cancel', async () => page.keyboard.press('Enter')], ['a click on Cancel', async () => page.click('.inline-confirm__cancel')]]) {
        await page.focus(trig); await page.keyboard.press('Enter'); await page.waitForTimeout(400);
        const a1 = await active(page);
        check('withdraw', `opening the confirm puts focus on Cancel (${how})` + tag, a1 && /cancel/.test(a1.id), JSON.stringify(a1));
        await act(); await page.waitForTimeout(500);
        const a2 = await active(page);
        check('withdraw', `${how} closes the confirm and focus returns to the trigger` + tag, a2 && a2.id === 'withdraw-application-trigger' && !(await page.locator('.inline-confirm').count()), JSON.stringify(a2));
      }
      check('withdraw', 'cancelling sent no withdraw request' + tag, reqs.length === 0, reqs.join(','));
    } else check('withdraw', 'withdraw trigger present for the pending applicant' + tag, false);
    await ctx.close();
  }
  // ---- Wiki History and Edit (Player01) -----------------------------------------
  {
    const ctx = await makeContext(browser, 'member', width, 'dark'); const page = await ctx.newPage();
    await open(page, '/wiki/tank-basics');
    const hist = 'button[aria-controls$="-history-panel"]';
    await page.focus(hist); await page.keyboard.press('Enter'); await page.waitForTimeout(700);
    const h = await page.evaluate(() => { const a = document.activeElement; const r = a.getBoundingClientRect(); const bar = document.querySelector('.d1-top').getBoundingClientRect(); const tab = document.querySelector('.site-tab-bar'); const tb = tab && getComputedStyle(tab).display !== 'none' ? tab.getBoundingClientRect().top : innerHeight; return { tag: a.tagName, text: a.textContent.trim(), top: Math.round(r.top), bottom: Math.round(r.bottom), barBottom: Math.round(bar.bottom), limit: Math.round(tb), scrollY: Math.round(scrollY) }; });
    check('wiki', 'Enter on History moves focus to the Revision history heading' + tag, h.tag === 'H2' && /Revision history/.test(h.text), JSON.stringify(h));
    check('wiki', 'the heading is in view, under the top bar and above the tab bar' + tag, h.top >= h.barBottom && h.bottom <= h.limit, JSON.stringify(h));
    const exp = await page.getAttribute(hist, 'aria-expanded');
    check('wiki', 'History reports aria-expanded=true' + tag, exp === 'true', exp);
    await page.focus(hist); await page.keyboard.press('Enter'); await page.waitForTimeout(400);
    const a3 = await active(page);
    check('wiki', 'closing History keeps focus on the button' + tag, a3 && /History/.test(a3.text) && (await page.getAttribute(hist, 'aria-expanded')) === 'false', JSON.stringify(a3));
    const edit = page.locator('button[id$="-edit"]');
    if (await edit.count()) {
      await edit.focus(); await page.keyboard.press('Enter'); await page.waitForTimeout(600);
      const e = await page.evaluate(() => { const a = document.activeElement; return { tag: a.tagName, start: a.selectionStart, end: a.selectionEnd, scrollTop: a.scrollTop, h: Math.round(a.getBoundingClientRect().height), first: (a.value || '').slice(0, 20) }; });
      check('wiki', 'Edit focuses the text area at its first line' + tag, e.tag === 'TEXTAREA' && e.start === 0 && e.end === 0 && e.scrollTop === 0, JSON.stringify(e));
      await page.keyboard.press('Tab'); await page.keyboard.press('Tab'); await page.keyboard.press('Tab');
      const c = await active(page);
      if (c && /Cancel/.test(c.text)) { await page.keyboard.press('Enter'); await page.waitForTimeout(500); const a4 = await active(page); check('wiki', 'Cancel returns focus to Edit' + tag, a4 && /-edit$/.test(a4.id), JSON.stringify(a4)); }
      else check('wiki', 'Cancel reachable by Tab from the text area' + tag, false, JSON.stringify(c));
    } else check('wiki', 'Edit button present for a member' + tag, false);
    await ctx.close();
  }
  // ---- Apply error summary ---------------------------------------------------
  {
    const ctx = await makeContext(browser, 'visitor', width, 'dark'); const page = await ctx.newPage();
    await open(page, '/apply');
    const hasForm = await page.locator('.apply-form').count();
    check('apply', 'form is shown to a signed-in account with no application' + tag, hasForm === 1);
    if (hasForm) {
      // Tab to the submit button from the last field, then Enter.
      await page.focus('#apply-message'); await page.keyboard.press('Tab');
      const a0 = await active(page);
      check('apply', 'Tab from the message field lands on the submit button' + tag, a0 && a0.tag === 'BUTTON' && /Send application/.test(a0.text), JSON.stringify(a0));
      await page.keyboard.press('Enter'); await page.waitForTimeout(500);
      const sum = await page.evaluate(() => { const s = document.getElementById('apply-error-summary'); return s ? { focused: document.activeElement === s, role: s.getAttribute('role'), links: [...s.querySelectorAll('a')].map(a => ({ href: a.getAttribute('href'), text: a.textContent.trim() })) } : null; });
      check('apply', 'submitting the empty form shows the error summary' + tag, sum && sum.links.length > 0, JSON.stringify(sum));
      check('apply', 'focus moves to the error summary after submit' + tag, sum && sum.focused, JSON.stringify(sum && { focused: sum.focused, active: await active(page) }));
      for (let i = 0; i < (sum ? sum.links.length : 0); i++) {
        const link = page.locator('#apply-error-summary a').nth(i);
        const href = await link.getAttribute('href'); const id = href.slice(1);
        await link.focus(); await page.keyboard.press('Enter'); await page.waitForTimeout(300);
        const res = await page.evaluate(id => { const a = document.activeElement; const t = document.getElementById(id); const inside = !!(t && a && (a === t || t.contains(a))); const r = a.getBoundingClientRect(); const tab = document.querySelector('.site-tab-bar'); const lim = tab && getComputedStyle(tab).display !== 'none' ? tab.getBoundingClientRect().top : innerHeight; const er = (document.getElementById(id + '-err') || a).getBoundingClientRect(); return { activeId: a.id, activeTag: a.tagName, inside, exact: a === t, inView: r.top >= 0 && r.bottom <= lim, errInView: er.bottom <= lim && er.top >= 0, top: Math.round(r.top), bottom: Math.round(r.bottom), errBottom: Math.round(er.bottom), lim: Math.round(lim), invalid: (t && (t.getAttribute('aria-invalid') || (t.querySelector('[aria-invalid]') || { getAttribute() {} }).getAttribute('aria-invalid'))) || null, describedby: t && t.getAttribute('aria-describedby') }; }, id);
        check('apply', `summary link ${i + 1} (${href}) moves focus to the invalid field` + tag, res.inside, JSON.stringify(res));
        check('apply', `the field focused from ${href} is in view` + tag, res.inView, JSON.stringify(res));
        check('apply', `the inline error of ${href} is in view too` + tag, res.errInView, JSON.stringify(res));
      }
      const inline = await page.evaluate(() => ['apply-games-err', 'apply-role-err', 'apply-message-err'].map(id => { const e = document.getElementById(id); return id + '=' + (e ? e.textContent.trim().slice(0, 50) : 'MISSING'); }));
      check('apply', 'each invalid field has its inline error text' + tag, inline.every(s => !s.endsWith('MISSING')), inline.join(' | '));
    }
    await ctx.close();
  }
  // ---- Patch notes rows --------------------------------------------------------
  {
    const ctx = await makeContext(browser, 'guest', width, 'dark'); const page = await ctx.newPage();
    await open(page, '/patch-notes');
    await page.waitForSelector('.pn-row-head');
    const row = n => page.locator('.pn-row-head').nth(n);
    const st = async n => ({ exp: await row(n).getAttribute('aria-expanded'), controls: await row(n).getAttribute('aria-controls'), body: await page.evaluate(n => !!document.querySelectorAll('.pn-row')[n].querySelector('.pn-row-body'), n) });
    const first0 = await st(0), second0 = await st(1);
    check('patch notes', 'latest row is open on load, the next one closed' + tag, first0.exp === 'true' && second0.exp === 'false', JSON.stringify({ first0, second0 }));
    await row(1).focus(); await page.keyboard.press('Enter'); await page.waitForTimeout(300);
    const second1 = await st(1);
    const ctrlOk = second1.controls && await page.evaluate(id => !!document.getElementById(id), second1.controls);
    check('patch notes', 'Enter opens a closed row: aria-expanded false -> true and the body renders' + tag, second1.exp === 'true' && second1.body, JSON.stringify(second1));
    check('patch notes', 'open row: aria-controls names the rendered body' + tag, ctrlOk, second1.controls);
    const a = await active(page);
    check('patch notes', 'focus stays on the row button after toggling' + tag, a && /pn-row-head/.test(a.cls), JSON.stringify(a));
    await page.keyboard.press('Enter'); await page.waitForTimeout(300);
    const second2 = await st(1);
    check('patch notes', 'Enter again closes it: aria-expanded true -> false' + tag, second2.exp === 'false' && !second2.body, JSON.stringify(second2));
    await page.keyboard.press('Space'); await page.waitForTimeout(300);
    check('patch notes', 'Space toggles too' + tag, (await st(1)).exp === 'true');
    // Hero rows inside the open latest patch.
    const hero = page.locator('.pn-row').first().locator('.pn-hero-head').first();
    if (await hero.count()) {
      const h0 = await hero.getAttribute('aria-expanded');
      await hero.focus(); await page.keyboard.press('Enter'); await page.waitForTimeout(300);
      const h1 = await hero.getAttribute('aria-expanded');
      check('patch notes', 'hero row toggles with Enter and aria-expanded flips' + tag, h0 !== h1 && ['true', 'false'].includes(h1), `${h0} -> ${h1}`);
    } else check('patch notes', 'hero row present in the open patch' + tag, false, 'no .pn-hero-head');
    // Expand all by keyboard.
    await page.focus('.pn-expand'); await page.keyboard.press('Enter'); await page.waitForTimeout(400);
    const all = await page.$$eval('.pn-row-head', bs => bs.map(b => b.getAttribute('aria-expanded')));
    const label = (await page.textContent('.pn-expand')).trim();
    check('patch notes', 'Expand all by Enter opens every row and the label flips' + tag, all.every(v => v === 'true') && /Collapse/.test(label), JSON.stringify({ all, label }));
    await ctx.close();
  }
  // ---- Code block in an article ---------------------------------------------------
  for (const [name, path] of [['blog post', '/blog/scrim-review-three-weeks'], ['wiki page', '/wiki/tank-basics']]) {
    const ctx = await makeContext(browser, 'guest', width, 'dark'); const page = await ctx.newPage();
    await open(page, path);
    const pre = page.locator('.md-body pre').first();
    if (!(await pre.count())) { check('code block', name + ': code block present' + tag, false, 'no pre in .md-body'); await ctx.close(); continue; }
    const info = await pre.evaluate(el => ({ tabindex: el.getAttribute('tabindex'), role: el.getAttribute('role'), label: el.getAttribute('aria-label'), scrolls: el.scrollWidth > el.clientWidth, cls: el.className }));
    check('code block', name + ': pre has tabindex=0 and a name' + tag, info.tabindex === '0' && !!info.label, JSON.stringify(info));
    // Reach it with Tab only, starting from the top of the page.
    await page.evaluate(() => { document.activeElement && document.activeElement.blur(); window.scrollTo(0, 0); });
    let reached = false, stops = 0;
    for (; stops < 80 && !reached; stops++) { await page.keyboard.press('Tab'); reached = await page.evaluate(() => !!document.activeElement && document.activeElement.matches('.md-body pre')); }
    check('code block', name + ': reachable with Tab' + tag, reached, `after ${stops} Tab presses`);
    if (reached) {
      const ring = await page.evaluate(() => { const s = getComputedStyle(document.activeElement); return { outline: s.outlineStyle + ' ' + s.outlineWidth + ' ' + s.outlineColor, shadow: s.boxShadow.slice(0, 60) }; });
      check('code block', name + ': focused block shows a focus ring' + tag, (ring.outline && !/^none|\b0px\b/.test(ring.outline)) || (ring.shadow && ring.shadow !== 'none'), JSON.stringify(ring));
      if (info.scrolls) {
        const before = await pre.evaluate(el => el.scrollLeft);
        for (let i = 0; i < 4; i++) await page.keyboard.press('ArrowRight');
        await page.waitForTimeout(300);
        const after = await pre.evaluate(el => el.scrollLeft);
        check('code block', name + ': ArrowRight scrolls the focused block' + tag, after > before, `${before} -> ${after}`);
      } else results.push({ group: 'code block', name: name + ': block does not overflow at this width, arrow scroll not tested' + tag, pass: true, detail: 'n/a' });
    }
    const table = page.locator('.md-body .md-table-wrap').first();
    if (await table.count()) { const t = await table.evaluate(el => ({ tabindex: el.getAttribute('tabindex'), label: el.getAttribute('aria-label') })); check('code block', name + ': table wrapper is focusable too' + tag, t.tabindex === '0' && !!t.label, JSON.stringify(t)); }
    await ctx.close();
  }
}
await browser.close();
fs.writeFileSync(REPORT_DIR + '/report-keys.json', JSON.stringify(results, null, 1));
const fails = results.filter(r => !r.pass);
console.log(`\n${results.length - fails.length} pass, ${fails.length} fail`);
process.exit(fails.length ? 1 : 0);
