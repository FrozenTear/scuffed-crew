# Stat-tracker test fixtures (local only)

Drop native-resolution scoreboard screenshots here to validate the capture
pipeline. Files are gitignored (copyrighted game captures — do not commit).

Run the extraction pipeline against any image:

```
cargo run -p scuffed-stat-tracker --example extract -- <absolute-path-to.png> [team_size]
```

It prints detected team size, outcome (color + header-text), the player-row
portrait match, the per-cell OCR for every row (E A D DMG HLG MIT with
confidences), and the final parsed match. The optional `team_size` arg (5 or 6)
forces team size to isolate detection bugs from OCR bugs.

Most useful frames to capture:
- A native-res (1080p+) **post-match scoreboard** in default blue/red colors.
- One in **custom team colors** (e.g. purple/gold) — exercises the color paths.
- A **6v6** board with empty "WAITING FOR PLAYER" rows — exercises team-size detection.
