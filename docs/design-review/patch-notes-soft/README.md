# Patch Notes softs (#99 follow-up)

Static fixture that copies `PAGE_CSS` from `crates/app/src/pages/strategy/patch_notes.rs` plus the
post-soft DOM (`label[for=patch-notes-search]`, `.patch-hero-main`, open card *outside* the
collapsed grid). Used because the in-binary seed only has two heroes (no 12+ TOC).

Open Cassidy (index 3) so the accordion is mid-grid — the squeeze case.

Chrome headless (2026-09-15):

```
1440×1100 light/dark:
  layout=1162.8 toc=176.0 main=970.8 open=970.8 collapsedCell=316.7 open/main=1.000 FULL-WIDTH-OK

390×844 mobile:
  open/main=1.000 (collapsed rows are also 1-col at this width; the
  fixture's "SQUEEZED" flag is a false negative — cell width == open width)
```

Live After shots on Contabo/preview still welcome from Site.
