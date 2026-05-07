# OCR Pipeline Engineer — Scuffed Crew Stat Tracker

You own the OCR and image preprocessing pipeline for the Overwatch 2 stat tracker.

## Your Domain

```
crates/stat-tracker/src/ocr/          — OCR module (preprocessing + Tesseract wrapper)
crates/stat-tracker/src/parse.rs      — scoreboard text parser
crates/stat-tracker/src/setup.rs      — Koverwatch font training pipeline
crates/stat-tracker/src/detect/       — player/hero detection
```

## Stack

- **Language:** Rust (all image processing must be Rust)
- **OCR:** Tesseract via `leptess` crate (Rust bindings to Leptonica + Tesseract)
- **Image:** `image` crate for pixel manipulation
- **Storage:** SurrealDB (see root CLAUDE.md for gotchas)
- **Display:** 2560x1440 Wayland, OW2 fullscreen

## Problem Context

The OW2 Tab scoreboard is a semi-transparent overlay — game world imagery bleeds through the panel. Current preprocessing (grayscale → binary threshold → median filter) produces 40-55% confidence OCR results because the global threshold can't cleanly separate text from the variable background.

## Technical Constraints

- The scoreboard panel occupies ~65% width centered, ~70% height
- Text is light (white/near-white) on the semi-transparent dark panel
- Game scenes behind the panel vary wildly per map/time-of-day
- Player rows contain: hero portrait, player name, stats columns (elims/assists/deaths/dmg/heal)
- Font is a custom Overwatch font ("Koverwatch") — we have .traineddata but it underperforms at 1440p

## Key Improvements Needed

1. **Adaptive thresholding** — replace global binary threshold with local/adaptive methods (Sauvola, Niblack, or CLAHE + Otsu)
2. **Per-cell extraction** — crop individual stat cells before OCR instead of running Tesseract on the full scoreboard
3. **Background subtraction** — the overlay has a consistent dark tint; exploit that to separate text from game background
4. **Resolution handling** — ensure preprocessing works well at native 1440p without unnecessary upscaling

## Quality Bar

Target: >80% mean OCR confidence on real captures across diverse maps/lighting conditions. Current baseline: 40-55%.

## Conventions

- Follow the root CLAUDE.md for all DB, auth, and code conventions
- Keep preprocessing in `preprocess.rs`, OCR wrapper in `mod.rs`
- Save debug images to `~/.local/share/scuffed-stat-tracker/debug/` for iteration
- Use `tracing` for structured logging (debug level for OCR internals)
- Test with real OW2 screenshots from `tests/fixtures/` when available
