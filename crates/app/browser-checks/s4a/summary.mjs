// Groups report-audit.json failures by page, deduplicated across role, width
// and theme. Prints text and writes report-audit-summary.json.
import fs from 'node:fs';
import { REPORT_DIR as DIR } from './lib.mjs';
const report = JSON.parse(fs.readFileSync(DIR + '/report-audit.json', 'utf8'));
// Expected noise: guests get 401 from /api/auth/me, and the fake relay answers 502.
// The two missing-record pages are meant to get a 404 from the API.
const expectedHttp = (v, h) => (v.page === 'blog-failed' && h === '500 GET /api/articles') || (v.page === 'blog-post-404' && h === '404 GET /api/articles/no-such-post') || (v.page === 'wiki-404' && h === '404 GET /api/wiki/no-such-topic') || (/^401 GET \/api\/auth\/me$/.test(h) && v.role === 'guest') || /^502 /.test(h);
const pages = {};
const add = (page, kind, item, v) => {
  const p = pages[page] ??= {}; const k = p[kind] ??= new Map();
  const e = k.get(item) ?? { roles: new Set(), widths: new Set(), themes: new Set() };
  e.roles.add(v.role); e.widths.add(v.w); e.themes.add(v.theme); k.set(item, e);
};
for (const v of Object.values(report)) {
  const pg = v.page;
  if (v.overflowX > 0) add(pg, 'overflow', `document ${v.overflowX}px wider than viewport: ${v.wide.join(' | ') || '(no unclipped element found)'}`, v);
  for (const u of v.unnamed) add(pg, 'unnamed control', u, v);
  // An aria-disabled control is an inactive component: WCAG 1.4.3 exempts it. Listed as a note, not a failure.
  for (const c of v.contrast) add(pg, c.disabled ? 'note: disabled control under 4.5 (exempt)' : 'contrast < 4.5', `${c.sel} "${c.text}" fg ${c.fg} on ${c.bg} = ${c.ratio} (${c.px}px/${c.weight}${c.large ? ', large text' : ''}${c.disabled ? ', disabled' : ''}${c.overImage ? ', over image/gradient' : ''}${c.alpha ? ', alpha ' + c.alpha : ''})`, v);
  for (const s of v.small) add(pg, 'text < 12px', `${s.sel} "${s.text}" ${s.px}px weight ${s.weight}${s.upper ? ' uppercase' : ''}`, v);
  for (const t of v.targets) add(pg, 'target < 44px', `${t.sel} "${t.text}" ${t.w}x${t.h}`, v);
  for (const d of v.dupIds) add(pg, 'duplicate id', d, v);
  if (v.h1 !== 1) add(pg, 'h1 count', `h1=${v.h1} ${JSON.stringify(v.h1Text)}`, v);
  if (v.mains !== 1) add(pg, 'main count', `main=${v.mains}`, v);
  if (v.skeletons) add(pg, 'skeleton in shot', `${v.skeletons} skeleton nodes`, v);
  const http = v.http.filter(h => !expectedHttp(v, h));
  for (const h of http) add(pg, 'unexpected http', h, v);
  // Console "Failed to load resource" lines only repeat the http list.
  for (const e of v.errs) if (!/Failed to load resource/.test(e)) add(pg, 'console error', e, v);
  if (v.errs.filter(e => /Failed to load resource/.test(e)).length > v.http.length) add(pg, 'console error', 'resource error with no matching response', v);
}
const out = {}; let total = 0;
for (const [pg, kinds] of Object.entries(pages)) {
  console.log('\n== ' + pg);
  out[pg] = {};
  for (const [kind, items] of Object.entries(kinds)) {
    console.log('  ' + kind + ':');
    out[pg][kind] = [];
    for (const [item, e] of items) {
      const scope = `[${[...e.roles].join('/')}; ${[...e.widths].sort().join('/')}; ${[...e.themes].sort().join('/')}]`;
      console.log('    - ' + item + ' ' + scope); out[pg][kind].push({ item, roles: [...e.roles], widths: [...e.widths], themes: [...e.themes] }); if (!kind.startsWith('note:')) total++;
    }
  }
}
const clean = [...new Set(Object.values(report).map(v => v.page))].filter(p => !pages[p] || Object.keys(pages[p]).every(k => k.startsWith('note:')));
console.log(`\n${total} distinct failures on ${Object.values(pages).filter(k => Object.keys(k).some(x => !x.startsWith('note:'))).length} pages; clean pages: ${clean.join(', ') || 'none'}; captures: ${Object.keys(report).length}`);
fs.writeFileSync(DIR + '/report-audit-summary.json', JSON.stringify(out, null, 1));
