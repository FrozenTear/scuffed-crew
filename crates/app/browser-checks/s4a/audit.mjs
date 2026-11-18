// In-page audit, run with page.evaluate(audit). Returns plain data.
export const audit = () => {
  const cv = document.createElement('canvas'); cv.width = cv.height = 1;
  const cx = cv.getContext('2d', { willReadFrequently: true });
  const rgba = c => { cx.clearRect(0, 0, 1, 1); cx.fillStyle = '#000'; cx.fillStyle = c; cx.fillRect(0, 0, 1, 1); const d = cx.getImageData(0, 0, 1, 1).data; return [d[0], d[1], d[2], d[3] / 255]; };
  const lum = ([r, g, b]) => { const f = v => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
  const over = (top, under) => { const a = top[3]; return [0, 1, 2].map(i => top[i] * a + under[i] * (1 - a)).concat([1]); };
  const hex = c => '#' + c.slice(0, 3).map(v => Math.round(v).toString(16).padStart(2, '0')).join('');
  // Effective background: stack the translucent layers from the element up
  // to the first opaque ancestor, then the page canvas (white if nothing).
  const bgOf = el => {
    const layers = []; let image = false;
    for (let e = el; e; e = e.parentElement) {
      const s = getComputedStyle(e);
      if (s.backgroundImage && s.backgroundImage !== 'none') image = true;
      const c = rgba(s.backgroundColor);
      if (c[3] > 0) { layers.push(c); if (c[3] >= 0.999) break; }
    }
    let base = [255, 255, 255, 1];
    for (let i = layers.length - 1; i >= 0; i--) base = over(layers[i], base);
    return { bg: base, image };
  };
  const opacityOf = el => { let o = 1; for (let e = el; e; e = e.parentElement) o *= parseFloat(getComputedStyle(e).opacity || '1'); return o; };
  const vis = el => {
    const r = el.getBoundingClientRect(); const s = getComputedStyle(el);
    if (!(r.width > 0 && r.height > 0) || s.visibility === 'hidden' || s.display === 'none') return false;
    for (let e = el; e; e = e.parentElement) { const st = getComputedStyle(e); if (st.display === 'none' || st.visibility === 'hidden') return false; if (e.hasAttribute('hidden') || e.inert) return false; }
    return true;
  };
  // Visually hidden text (1px clip pattern) is for assistive tech, not sight.
  const srOnly = el => { for (let e = el; e && e !== document.body; e = e.parentElement) { const r = e.getBoundingClientRect(); const s = getComputedStyle(e); if ((r.width <= 1.5 || r.height <= 1.5) && s.overflow !== 'visible') return true; if (s.clipPath && s.clipPath.includes('inset(50%)')) return true; } return false; };
  const one = el => el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') + (el.className && typeof el.className === 'string' && el.className.trim() ? '.' + el.className.trim().split(/\s+/).join('.') : '');
  const desc = el => { const parts = [one(el)]; let p = el.parentElement; for (let i = 0; i < 2 && p && p.tagName !== 'MAIN' && p !== document.body; i++, p = p.parentElement) parts.unshift(one(p)); return parts.join(' > ').slice(0, 160); };
  const shellSel = '.skip-link, .d1-top, .site-subnav, .site-drawer, .site-tab-bar, .site-footer';
  const main = document.querySelector('main');
  const inMain = el => !!(main && main.contains(el));
  const text = s => (s || '').replace(/\s+/g, ' ').trim();
  const nameOf = el => {
    const lb = el.getAttribute('aria-labelledby');
    if (lb) { const t = text(lb.split(/\s+/).map(id => (document.getElementById(id) || {}).textContent || '').join(' ')); if (t) return t; }
    return text(el.getAttribute('aria-label')) || (el.labels && el.labels[0] && text(el.labels[0].textContent)) || text(el.textContent) ||
      text(el.getAttribute('title')) || text(el.getAttribute('placeholder')) || text((el.querySelector('img[alt]') || {}).alt) ||
      (el.tagName === 'INPUT' && /^(submit|button|reset)$/.test(el.type) ? text(el.value) : '');
  };
  const de = document.documentElement; const cw = de.clientWidth;
  const out = {
    url: location.pathname + location.search, title: document.title, width: innerWidth,
    overflowX: Math.max(0, de.scrollWidth - cw), wide: [], unnamed: [], contrast: [], small: [], targets: [],
    h1: document.querySelectorAll('h1').length, h1Text: [...document.querySelectorAll('h1')].map(h => text(h.textContent).slice(0, 60)),
    mains: document.querySelectorAll('main').length, dupIds: [], textChecked: 0, controlsChecked: 0, targetsChecked: 0, skeletons: document.querySelectorAll('.ui-skel-list, .ui-skel-row, .home-skel').length,
  };
  // Overflow: which elements stick out past the viewport without a clipping parent.
  if (out.overflowX > 0) {
    for (const el of document.querySelectorAll('body *')) {
      if (!vis(el)) continue; const r = el.getBoundingClientRect();
      if (r.right > cw + 1 && r.left < cw) {
        let clipped = false; for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) { if (getComputedStyle(p).overflowX !== 'visible') { clipped = true; break; } }
        if (!clipped) out.wide.push(`${desc(el)} right=${Math.round(r.right)}`);
      }
    }
    out.wide = out.wide.slice(0, 10);
  }
  // Controls with no accessible name (whole document; shell ones are tagged).
  for (const el of document.querySelectorAll('a, button, input, select, textarea, [role=button], [tabindex]')) {
    if (el.tagName === 'INPUT' && el.type === 'hidden') continue;
    if (!vis(el)) continue;
    out.controlsChecked++;
    if (!nameOf(el)) out.unnamed.push(desc(el) + (el.closest(shellSel) ? ' [shell]' : '') + (el.getAttribute('tabindex') === '-1' ? ' [tabindex=-1]' : ''));
  }
  // Text contrast and the type floor, inside main only.
  if (main) {
    const seen = new Set();
    const w = document.createTreeWalker(main, NodeFilter.SHOW_TEXT);
    for (let n = w.nextNode(); n; n = w.nextNode()) {
      const t = text(n.textContent); if (!t) continue;
      const el = n.parentElement; if (!el || seen.has(el)) continue; seen.add(el);
      if (['SCRIPT', 'STYLE', 'NOSCRIPT', 'OPTION'].includes(el.tagName)) continue;
      if (!vis(el) || srOnly(el)) continue;
      const s = getComputedStyle(el); out.textChecked++;
      const { bg, image } = bgOf(el);
      const raw = rgba(s.color); const alpha = raw[3] * opacityOf(el);
      const fg = over([raw[0], raw[1], raw[2], alpha], bg);
      const l1 = lum(fg), l2 = lum(bg); const ratio = (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
      const px = parseFloat(s.fontSize); const weight = parseInt(s.fontWeight, 10) || 400;
      if (ratio < 4.5) {
        const large = px >= 24 || (px >= 18.66 && weight >= 700);
        const disabled = !!el.closest(':disabled, [aria-disabled="true"]');
        out.contrast.push({ sel: desc(el), fg: hex(fg), bg: hex(bg), ratio: +ratio.toFixed(2), text: t.slice(0, 40), px, weight, ...(large ? { large: true } : {}), ...(disabled ? { disabled: true } : {}), ...(image ? { overImage: true } : {}), ...(alpha < 0.999 ? { alpha: +alpha.toFixed(2) } : {}) });
      }
      const upper = s.textTransform === 'uppercase';
      if (px < 11 || (px < 12 && !(upper || weight >= 600))) out.small.push({ sel: desc(el), px, weight, upper, text: t.slice(0, 40) });
    }
    // Targets under 44px at phone width. Inline links in running text are exempt.
    if (innerWidth <= 820) {
      for (const el of main.querySelectorAll('a, button, input, select, textarea, summary, [role=button], [role=tab], [role=checkbox], [role=switch]')) {
        if (el.tagName === 'INPUT' && el.type === 'hidden') continue;
        let box = el;
        // A visually hidden or tiny native input is clicked through its label.
        if (el.tagName === 'INPUT' && /^(checkbox|radio)$/.test(el.type) && el.labels && el.labels[0]) box = el.labels[0];
        if (!vis(box)) continue;
        // Stretched link: a positioned ::before/::after with inset 0 makes the
        // nearest positioned ancestor the real target.
        for (const pe of ['::after', '::before']) {
          const ps = getComputedStyle(el, pe);
          if (ps.content !== 'none' && ps.position === 'absolute' && parseFloat(ps.top) === 0 && parseFloat(ps.left) === 0 && getComputedStyle(el).position === 'static') {
            let anc = el.parentElement; while (anc && getComputedStyle(anc).position === 'static') anc = anc.parentElement;
            if (anc) box = anc;
          }
        }
        if (el.tagName === 'A' && getComputedStyle(el).display.startsWith('inline')) {
          const blk = el.closest('p, li, td, th, dd, blockquote, figcaption');
          if (blk && text(blk.textContent).length > text(el.textContent).length + 1) continue;
        }
        const r = box.getBoundingClientRect(); out.targetsChecked++;
        if (r.width < 43.5 || r.height < 43.5) out.targets.push({ sel: desc(el), w: Math.round(r.width), h: Math.round(r.height), text: nameOf(el).slice(0, 40) });
      }
    }
  }
  const ids = {}; for (const e of document.querySelectorAll('[id]')) ids[e.id] = (ids[e.id] || 0) + 1;
  out.dupIds = Object.keys(ids).filter(k => ids[k] > 1);
  return out;
};
