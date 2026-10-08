# Scuffed Vision baseline fixtures (PR 158)

Cut from the streamer's own 2026-10-07/08 captures (6v6, 2560x1440). `native`
files are at the saved 1664x1007 scoreboard-crop scale. `s075` files come from
the same crop resized 0.75x with Lanczos, which is the 1248x756 crop a
1920x1080 frame gives. No player names: single stat cells, or boards whose
name column (and everything outside the stat table) is filled with a solid
colour. `tests/vision_baseline_fixtures.rs` reads all of them.

## Bottom-row cells (finding 1)

Each cell is exactly what `crop_player_row` plus `crop_stat_cell` cut on that
frame. The bottom row runs past the crop, so these cells are 38 px tall at
native (27 px at 0.75x) instead of 53 (40).

| File | Frame | Row, column | Expected |
|---|---|---|---|
| `native_bottom_row_assists_white8_a.png` | `000324` | 11, A | `8` |
| `native_bottom_row_assists_white8_b.png` | `000330` | 11, A | `8` |
| `native_bottom_row_assists_white8_c.png` | `000338` | 11, A | `8` |
| `native_bottom_row_mitigation_white183.png` | `001449` | 11, MIT | `183` |
| `s075_bottom_row_assists_white8.png` | `000338` | 11, A | `8` |
| `s075_bottom_row_mitigation_white183.png` | `001449` | 11, MIT | `183` |

## 0.75x board (finding 1)

`s075_tab_board_stats_only.png` is frame `001449` at 0.75x. The header strip
and the stat table are untouched; portraits, names and the career panel are
filled. Read as 6v6, it must keep these cells main reads:

| Row | Column | Expected |
|---|---|---|
| 0 | E | `6` |
| 1 | A | `0` |
| 1 | H | `0` |
| 2 | E | `5` |
| 3 | A | `1` |
| 3 | H | `0` |
| 3 | MIT | `84` |
| 5 | E | `8` |
| 7 | E | `5` |
| 8 | A | `1` |

## Dim zeros (finding 2)

Every file is a dim grey `0` that only the geometric check recovers. Expected
value `0`, not suspect, and a confidence of at least 75. The measured hole
extent and centroid offset, and the confidence they give, are:

| File | Frame | Row, column | Extent % | Offset % | Confidence |
|---|---|---|---|---|---|
| `native_purple_deaths_dim0.png` | `001150` | 1, D | 40 | 4 | 76 |
| `native_purple_healing_dim0.png` | `001150` | 3, H | 45 | 1 | 94 |
| `native_yellow_mitigation_dim0.png` | `235953` | 10, MIT | 40 | 3 | 76 |
| `native_yellow_mitigation_dim0_wide_counter.png` | `000507` | 7, MIT | 55 | 2 | 94 |
| `s075_purple_mitigation_dim0.png` | `001150` | 1, MIT | 42 | 2 | 85 |
| `s075_yellow_elims_dim0.png` | `001150` | 9, E | 60 | 0 | 95 |
| `s075_yellow_healing_dim0.png` | `001626` | 6, H | 42 | 0 | 87 |

## Post-game table (finding 3)

`native_postgame_table_names_blanked.png` and
`s075_postgame_table_names_blanked.png` are frame `235921`, the centred
end-of-match table with placeholder portraits. Names and titles are filled
with each scanline's own row colour, and everything outside the table is
filled. Portraits, row bands and the stat columns are untouched.

Filling the names removes the per-row dips that team-size detection reads,
so these files cannot stand in for the real frame's row scan. The tests use
the row scan measured on the real frame (native: 4 dips, dip pitch 0.0745,
spectral pitch 0.1023; 0.75x: 4 dips, 0.0754, 0.1019) and read the table at
the size it gives. That must be 6. Expected values, rows 0-5 purple and
6-11 yellow, columns E, A, D, DMG, H, MIT:

```
12  3 3 5480  950 2347
 7  1 7 3789    0   82
 7  8 7 1858 3387  110
 7 15 6 1545 5407    0
15  0 5 7283  332   24
 9  0 5 4922  987 5785
21  8 3 7161 1540 6914
15  1 4 4812  142    0
16  5 5 5643  952 1685
14  8 2 1762 1605  123
15 15 3 3448 6595    0
 6  5 4 2430 1382  203
```
