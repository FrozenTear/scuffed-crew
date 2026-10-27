//! Styles for the Heroes page, moved unchanged from the old single file
//! apart from heading margins and the role tinted portrait fallback.

pub(super) const PAGE_CSS: &str = r#"
.heroes-page { display:flex; height:100%; min-height:calc(100vh - 60px); }

/* Left panel */
.heroes-left { width:340px; min-width:280px; border-right:1px solid var(--border); overflow-y:auto; padding:1.25rem; background:var(--surface); flex-shrink:0; }
.heroes-search { width:100%; padding:0.5rem 0.75rem; border:1px solid var(--border); border-radius:6px; background:var(--surface-2); color:var(--text); font-family:var(--font-body); font-size:0.85rem; margin-bottom:1.25rem; transition:border-color 0.2s; }
.heroes-search:focus { border-color:var(--accent); }
.heroes-search::placeholder { color:var(--text-3); }
.heroes-role-heading { margin:1rem 0 0.5rem; font-family:var(--font-head); font-size:0.75rem; font-weight:700; text-transform:uppercase; letter-spacing:0.08em; padding-bottom:0.35rem; border-bottom:1px solid var(--border); }
.heroes-role:first-of-type .heroes-role-heading { margin-top:0; }
.heroes-grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(min(90px,100%),1fr)); gap:0.5rem; margin-bottom:0.75rem; }
.heroes-page .hero-btn { display:flex; flex-direction:column; align-items:center; gap:0.25rem; padding:0.5rem 0.25rem; border:1px solid var(--border); border-radius:8px; background:var(--surface-2); cursor:pointer; transition:border-color 0.15s,background 0.15s,transform 0.1s; text-align:center; }
.heroes-page .hero-btn:hover { border-color:var(--accent-soft); background:var(--surface-2); transform:translateY(-1px); }
.heroes-page .hero-btn.selected { border-color:var(--accent); background:var(--accent-soft); box-shadow:0 0 0 1px var(--accent); }
.hero-btn-portrait { width:40px; height:40px; border-radius:50%; overflow:hidden; background:var(--surface-2); display:flex; align-items:center; justify-content:center; flex-shrink:0; }
.hero-btn-portrait img { width:100%; height:100%; object-fit:cover; }
.hero-btn-initial { font-family:var(--font-head); font-size:1.1rem; color:inherit; line-height:1; }
.hero-btn-name { font-family:var(--font-head); font-size:0.7rem; font-weight:600; color:var(--text); line-height:1.2; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; max-width:100%; }

/* Right panel */
.heroes-right { flex:1; overflow-y:auto; padding:2rem; min-width:0; }
.heroes-placeholder { display:flex; align-items:center; justify-content:center; height:100%; min-height:400px; color:var(--text-3); font-size:1rem; font-family:var(--font-head); text-transform:uppercase; letter-spacing:0.06em; }

/* Detail header */
.hero-detail-header { display:flex; align-items:center; gap:1.25rem; margin-bottom:1.75rem; }
.hero-detail-portrait { width:80px; height:80px; border-radius:12px; overflow:hidden; background:var(--surface-2); display:flex; align-items:center; justify-content:center; flex-shrink:0; }
.hero-detail-portrait img { width:100%; height:100%; object-fit:cover; }
.hero-detail-portrait-initial { font-family:var(--font-head); font-size:2rem; color:inherit; }
.heroes-page .hero-detail-info { display:flex; flex-direction:column; gap:0.35rem; min-width:0; }
.hero-detail-name { margin:0; font-family:var(--font-head); font-size:2rem; color:var(--text); letter-spacing:2px; line-height:1; }
.hero-detail-role-badge { display:inline-block; font-size:0.65rem; padding:0.15rem 0.6rem; border-radius:999px; font-weight:700; text-transform:uppercase; letter-spacing:0.05em; width:fit-content; }

/* HP breakdown */
.hero-hp-section { margin-bottom:2rem; }
.hero-hp-label { font-family:var(--font-head); font-size:0.75rem; font-weight:600; text-transform:uppercase; letter-spacing:0.06em; color:var(--text-3); margin-bottom:0.5rem; }
.hero-hp-bar { display:flex; height:18px; border-radius:4px; overflow:hidden; background:var(--surface-2); margin-bottom:0.4rem; }
.hero-hp-segment { display:flex; align-items:center; justify-content:center; font-size:0.6rem; font-weight:700; font-family:var(--font-mono); color:var(--bg); min-width:24px; }
.hero-hp-legend { display:flex; flex-wrap:wrap; gap:0.35rem 1rem; font-size:0.7rem; color:var(--text-2); font-family:var(--font-mono); }
.hero-hp-legend span { display:flex; align-items:center; gap:0.3rem; }
.hero-hp-dot { width:8px; height:8px; border-radius:2px; display:inline-block; }

/* Abilities */
.hero-detail-pending { font-size:0.8rem; color:var(--text-3); }
.hero-abilities-title { margin:0 0 0.75rem; font-family:var(--font-head); font-size:0.9rem; font-weight:700; text-transform:uppercase; letter-spacing:0.06em; color:var(--text); padding-bottom:0.4rem; border-bottom:1px solid var(--border); }
.hero-abilities-list { display:flex; flex-direction:column; gap:0.75rem; }
.ability-card { display:flex; gap:0.75rem; padding:0.85rem 1rem; background:var(--surface-2); border:1px solid var(--border); border-radius:8px; transition:border-color 0.15s; }
.ability-card:hover { border-color:var(--accent-soft); }
.ability-key-badge { flex-shrink:0; width:42px; height:34px; display:flex; align-items:center; justify-content:center; border-radius:5px; background:var(--surface-2); border:1px solid var(--border); font-family:var(--font-mono); font-size:0.65rem; font-weight:700; color:var(--text); text-transform:uppercase; letter-spacing:0.03em; }
.ability-body { flex:1; min-width:0; }
.ability-header { display:flex; align-items:baseline; gap:0.5rem; margin-bottom:0.25rem; }
.heroes-page .ability-name { font-family:var(--font-head); font-size:0.85rem; font-weight:700; color:var(--text); }
.heroes-page .ability-cooldown { font-family:var(--font-mono); font-size:0.65rem; color:var(--text-3); }
.ability-desc { margin:0; font-size:0.8rem; color:var(--text-2); line-height:1.5; }

/* States */
.heroes-loading, .heroes-empty { color:var(--text-3); text-align:center; padding:3rem 0; }
.heroes-error { color:var(--danger); text-align:center; padding:3rem 0; font-size:0.85rem; }

/* Responsive */
@media (max-width:720px) {
    .heroes-page { flex-direction:column; }
    .heroes-left { width:100%; min-width:unset; border-right:none; border-bottom:1px solid var(--border); max-height:45vh; }
    .heroes-right { padding:1.25rem; }
}
"#;
