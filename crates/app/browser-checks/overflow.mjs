// No horizontal page overflow on every page, per role and width, and the
// /admin/relay tables scroll inside their own wrappers (QA 1 on PR 693).
import { launch, makeContext, open, readState, reporter, settled } from './lib.mjs';

const { rec, finish } = reporter('overflow');
const seed = readState('seed.json');
const members = readState('members.json').data;
const strategy = readState('strategy.json');
const m0 = members[0].id;
const PUBLIC = ['/', '/members', '/members/' + m0, '/forum', '/forum/b/' + seed.board.slug, '/forum/t/' + seed.thread.id, '/news', '/blog', '/blog/season-kickoff', '/wiki', '/tournaments', '/events', '/scrims', '/leaderboards', '/stats', '/strategy', '/strategy/heroes', '/strategy/meta', '/patch-notes', '/strategy/patch-notes', '/strategy/editor', '/strategy/editor/' + strategy.id, '/login', '/apply', '/no/such/page'];
const SIGNED = ['/feed', '/polls', '/chat', '/community', '/strategy/my', '/profile/edit', '/dm', '/identity', '/reports', '/link', '/stats/tokens'];
const ADMIN = ['/admin', '/admin/members', `/admin/members/${m0}/matches`, '/admin/teams', '/admin/applications', '/admin/forum', '/admin/moderation', '/admin/games', '/admin/announcements', '/admin/articles', '/admin/patch-notes', '/admin/schedule', '/admin/matches', '/admin/tournaments', '/admin/reports', '/admin/hero-icons', '/admin/seasons', '/admin/relay', '/admin/audit-log', '/admin/settings'];

const browser = await launch();
for (const [role, widths, paths] of [['admin', [1440, 390, 360], [...PUBLIC, ...SIGNED, ...ADMIN]], ['guest', [390, 360], PUBLIC], ['member', [390], [...PUBLIC, ...SIGNED]]]) {
  for (const width of widths) {
    const ctx = await makeContext(browser, role, width); const page = await ctx.newPage(); const bad = []; const stuck = []; const errored = [];
    for (const p of paths) {
      try { await open(page, p); } catch (e) { bad.push(p + ' LOAD ' + e.message.slice(0, 60)); continue; }
      // Measure the loaded page, not its loading state (Vision nit 26).
      const left = await settled(page);
      if (left.pending.length) stuck.push(`${p} [${left.pending.join('; ')}]`);
      if (left.errored.length) errored.push(`${p} [${left.errored.join('; ')}]`);
      const m = await page.evaluate(() => ({ sw: document.documentElement.scrollWidth, bw: document.body.scrollWidth, iw: innerWidth }));
      if (m.sw > m.iw || m.bw > m.iw) bad.push(`${p} ${m.sw}/${m.bw} > ${m.iw}`);
    }
    rec(`${role} ${width}: no horizontal overflow on ${paths.length} pages`, bad.length === 0, bad.join(' | '));
    rec(`${role} ${width}: every page leaves its loading state within 10 s before it is measured`, stuck.length === 0, stuck.join(' | '));
    rec(`${role} ${width}: no page is left on a loading marker that reports an error`, errored.length === 0, errored.join(' | '));
    if (role === 'admin') {
      await open(page, '/admin/relay');
      // The tables arrive with /api/nostr/health: wait for them, do not count once.
      const arrived = await page.waitForFunction(() => document.querySelectorAll('table.data-table').length >= 2, null, { timeout: 10000 }).then(() => true, () => false);
      rec(`admin ${width}: /admin/relay shows its 2 tables within 10 s`, arrived);
      const r = await page.evaluate(() => ({ tables: document.querySelectorAll('table.data-table').length, wrapped: [...document.querySelectorAll('table.data-table')].every(t => t.parentElement.classList.contains('data-table-scroll')), sw: document.documentElement.scrollWidth, iw: innerWidth, main: getComputedStyle(document.querySelector('.admin-main')).overflowX }));
      rec(`admin ${width}: /admin/relay has its 2 tables, each in its own scroller, page ${r.sw} <= ${r.iw}`, r.tables === 2 && r.wrapped && r.sw <= r.iw && r.main === 'auto', JSON.stringify(r));
    }
    await ctx.close();
  }
}
await browser.close();
finish();
