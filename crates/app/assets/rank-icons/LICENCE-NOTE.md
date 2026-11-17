# Rank emblems: licence note

The nine PNG files in this folder are the official Overwatch rank emblems by
Blizzard Entertainment, Inc., kept **unmodified**, byte for byte as Blizzard
ships them on the official Overwatch site (the career profile rank emblems).
No per-file source URL was recorded when they were collected; the source for
every file is the official Overwatch site. `rank-champion.png` is Blizzard's
`Rank_UltimateTier` file under our tier name.

| File | Tier | Source |
| --- | --- | --- |
| `rank-bronze.png` | Bronze | Official Overwatch site, career profile |
| `rank-silver.png` | Silver | Official Overwatch site, career profile |
| `rank-gold.png` | Gold | Official Overwatch site, career profile |
| `rank-platinum.png` | Platinum | Official Overwatch site, career profile |
| `rank-emerald.png` | Emerald | Official Overwatch site, career profile |
| `rank-diamond.png` | Diamond | Official Overwatch site, career profile |
| `rank-master.png` | Master | Official Overwatch site, career profile |
| `rank-grandmaster.png` | Grandmaster | Official Overwatch site, career profile |
| `rank-champion.png` | Champion | Official Overwatch site, career profile (`Rank_UltimateTier`) |

## Terms

Used under Blizzard's fansite and community content terms: a personal,
non-exclusive, non-transferable, revocable licence for noncommercial use,
provided copyright and other notices are kept. This site is a
noncommercial community site.

- The notice is kept: every page that shows an emblem also shows, once, in
  small muted text: "Overwatch rank emblems © Blizzard Entertainment, Inc.
  Overwatch and Blizzard are trademarks of Blizzard Entertainment, Inc."
  (`credit()` in `crates/app/src/pages/stats/rank.rs`).
- The emblems are decorative, next to the tier name in words, and only
  scaled in CSS. They are never recoloured, cropped or edited.

## Do not edit these files

Do not edit, recompress, resize or replace these files. The test
`emblem_files_are_unchanged` in `crates/app/src/pages/stats/rank.rs` pins
each file's byte length and FNV-1a hash and fails on any change.

## Why they are in git, and the dx repack

- The root `.gitignore` ignores `*.png` (screenshots and captures). The
  emblems were force-added for that reason, and now have their own
  `!crates/app/assets/rank-icons/*.png` exception, like the favicons. They
  must be in the repo: `asset!()` fails the build on a checkout without them.
- `dx build --release` repacks every PNG it bundles losslessly, these files
  included (as it does the favicons): the shipped files differ in bytes, the
  decoded pixels are identical. FrozenTear accepted this repack. The files
  in the repo stay the unmodified originals.
