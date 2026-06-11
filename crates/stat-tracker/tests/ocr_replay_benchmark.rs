use image::DynamicImage;
use stat_tracker::detect::hero_portrait::detect_team_size;
use stat_tracker::ocr;
use stat_tracker::ocr::preprocess;

struct GroundTruth {
    name: &'static str,
    stats: [u32; 6], // E, A, D, DMG, H, MIT
}

struct ReplayGroundTruth {
    file: &'static str,
    team1: Vec<GroundTruth>,
    team2: Vec<GroundTruth>,
}

fn load_image(path: &str) -> DynamicImage {
    let full = format!(
        "{}/../../tests/fixtures/replays/{path}",
        env!("CARGO_MANIFEST_DIR")
    );
    image::open(&full).unwrap_or_else(|e| panic!("failed to open {full}: {e}"))
}

fn ground_truth_replays() -> Vec<ReplayGroundTruth> {
    vec![
        ReplayGroundTruth {
            file: "replay_01.png",
            team1: vec![
                GroundTruth {
                    name: "FROZEN",
                    stats: [6, 1, 5, 6000, 920, 3013],
                },
                GroundTruth {
                    name: "BITLO",
                    stats: [2, 8, 3, 1746, 5301, 475],
                },
                GroundTruth {
                    name: "DIGITAL",
                    stats: [4, 9, 4, 3214, 5373, 0],
                },
                GroundTruth {
                    name: "HELXZY",
                    stats: [9, 1, 2, 7807, 724, 512],
                },
                GroundTruth {
                    name: "INVINCIBLE",
                    stats: [6, 1, 4, 7271, 0, 0],
                },
                GroundTruth {
                    name: "PLANET",
                    stats: [9, 1, 2, 7561, 1029, 1551],
                },
            ],
            team2: vec![
                GroundTruth {
                    name: "AFTERYOU123",
                    stats: [10, 0, 2, 6530, 0, 400],
                },
                GroundTruth {
                    name: "AKSSU",
                    stats: [3, 11, 3, 1906, 6709, 0],
                },
                GroundTruth {
                    name: "DIOSASESINO",
                    stats: [11, 6, 1, 6683, 1158, 3058],
                },
                GroundTruth {
                    name: "EFE",
                    stats: [4, 6, 4, 1756, 7918, 1752],
                },
                GroundTruth {
                    name: "MCLOVIN",
                    stats: [12, 4, 1, 6048, 4963, 899],
                },
                GroundTruth {
                    name: "SIONY",
                    stats: [7, 0, 6, 3545, 305, 176],
                },
            ],
        },
        ReplayGroundTruth {
            file: "replay_02.png",
            team1: vec![
                GroundTruth {
                    name: "ANDIRR",
                    stats: [5, 7, 3, 1619, 3775, 0],
                },
                GroundTruth {
                    name: "BIOSSFA",
                    stats: [3, 5, 3, 799, 3957, 290],
                },
                GroundTruth {
                    name: "N1CK",
                    stats: [7, 0, 4, 2431, 695, 3813],
                },
                GroundTruth {
                    name: "P1NSHOOTER",
                    stats: [8, 5, 1, 2871, 0, 2627],
                },
                GroundTruth {
                    name: "SHIBA",
                    stats: [2, 0, 3, 2901, 0, 0],
                },
                GroundTruth {
                    name: "TK+OW",
                    stats: [4, 0, 3, 3622, 0, 0],
                },
            ],
            team2: vec![
                GroundTruth {
                    name: "FROZEN",
                    stats: [6, 5, 1, 3275, 438, 1415],
                },
                GroundTruth {
                    name: "ANTS",
                    stats: [10, 0, 3, 4016, 321, 0],
                },
                GroundTruth {
                    name: "BERU",
                    stats: [8, 8, 1, 1239, 2857, 0],
                },
                GroundTruth {
                    name: "CANONSCHIZO",
                    stats: [5, 3, 1, 1023, 2180, 0],
                },
                GroundTruth {
                    name: "CATLEYA",
                    stats: [9, 0, 3, 4013, 0, 5548],
                },
                GroundTruth {
                    name: "CIANG43",
                    stats: [4, 2, 3, 1995, 557, 1557],
                },
            ],
        },
        ReplayGroundTruth {
            file: "replay_03.png",
            team1: vec![
                GroundTruth {
                    name: "ALEXFORPS",
                    stats: [3, 8, 4, 3857, 10266, 4309],
                },
                GroundTruth {
                    name: "BENBO",
                    stats: [5, 6, 3, 2113, 7687, 73],
                },
                GroundTruth {
                    name: "DAVIDTITUSEN",
                    stats: [6, 0, 6, 4520, 0, 9137],
                },
                GroundTruth {
                    name: "ILLUSIVEMAN",
                    stats: [6, 2, 6, 5373, 201, 0],
                },
                GroundTruth {
                    name: "PRATICKTRUE",
                    stats: [7, 0, 6, 6454, 70, 23],
                },
                GroundTruth {
                    name: "SNAKEEYES",
                    stats: [7, 1, 6, 5333, 646, 7057],
                },
            ],
            team2: vec![
                GroundTruth {
                    name: "FROZEN",
                    stats: [18, 6, 3, 5178, 545, 2222],
                },
                GroundTruth {
                    name: "D1R1D1",
                    stats: [6, 17, 0, 1115, 7293, 253],
                },
                GroundTruth {
                    name: "IRONHUNTER",
                    stats: [23, 6, 2, 6151, 246, 8083],
                },
                GroundTruth {
                    name: "JALLABALLA",
                    stats: [21, 5, 3, 7309, 0, 0],
                },
                GroundTruth {
                    name: "SCHLAWG",
                    stats: [19, 0, 2, 8668, 1174, 0],
                },
                GroundTruth {
                    name: "CLAWG",
                    stats: [13, 17, 1, 3458, 7470, 666],
                },
            ],
        },
        ReplayGroundTruth {
            file: "replay_04.png",
            team1: vec![
                GroundTruth {
                    name: "FROZEN",
                    stats: [6, 14, 5, 2427, 4676, 0],
                },
                GroundTruth {
                    name: "ABDORPTED",
                    stats: [11, 3, 7, 5980, 0, 5046],
                },
                GroundTruth {
                    name: "GOOSEANGOOSE",
                    stats: [14, 7, 5, 4170, 770, 3559],
                },
                GroundTruth {
                    name: "MASHIRO",
                    stats: [5, 12, 5, 2887, 5165, 0],
                },
                GroundTruth {
                    name: "MEGALODON",
                    stats: [11, 3, 4, 6526, 0, 0],
                },
                GroundTruth {
                    name: "VASQUEZ",
                    stats: [12, 0, 5, 5320, 0, 0],
                },
            ],
            team2: vec![
                GroundTruth {
                    name: "FISKSAPARE",
                    stats: [15, 1, 4, 6373, 1437, 9614],
                },
                GroundTruth {
                    name: "ILLUSIVEMAN",
                    stats: [23, 7, 6, 7882, 0, 0],
                },
                GroundTruth {
                    name: "NIGELH",
                    stats: [16, 4, 4, 5277, 0, 227],
                },
                GroundTruth {
                    name: "ORTOPEDEN",
                    stats: [18, 0, 4, 6933, 0, 7299],
                },
                GroundTruth {
                    name: "PIR48",
                    stats: [8, 10, 3, 1249, 4556, 31],
                },
                GroundTruth {
                    name: "BABURON",
                    stats: [5, 21, 3, 989, 8242, 0],
                },
            ],
        },
        ReplayGroundTruth {
            file: "replay_05.png",
            team1: vec![
                GroundTruth {
                    name: "ATINA",
                    stats: [1, 3, 2, 718, 2860, 0],
                },
                GroundTruth {
                    name: "BUGG",
                    stats: [5, 2, 2, 2971, 45, 0],
                },
                GroundTruth {
                    name: "KOALAKING",
                    stats: [3, 0, 4, 1599, 0, 5166],
                },
                GroundTruth {
                    name: "QUILIO",
                    stats: [4, 0, 3, 3733, 0, 2656],
                },
                GroundTruth {
                    name: "SOMBRUH",
                    stats: [7, 2, 2, 2325, 4278, 0],
                },
                GroundTruth {
                    name: "ZIAIN",
                    stats: [2, 2, 4, 1173, 2035, 780],
                },
            ],
            team2: vec![
                GroundTruth {
                    name: "FROZEN",
                    stats: [10, 5, 2, 3250, 415, 992],
                },
                GroundTruth {
                    name: "AKSSU",
                    stats: [7, 7, 2, 1352, 1796, 0],
                },
                GroundTruth {
                    name: "CH3ARRY",
                    stats: [8, 7, 2, 1652, 1231, 0],
                },
                GroundTruth {
                    name: "VASQUEZ",
                    stats: [12, 1, 1, 4108, 0, 0],
                },
                GroundTruth {
                    name: "WASP",
                    stats: [14, 4, 0, 5254, 940, 1295],
                },
                GroundTruth {
                    name: "GENJI",
                    stats: [4, 5, 1, 776, 2188, 267],
                },
            ],
        },
    ]
}

fn load_all_frames() -> Vec<(String, DynamicImage)> {
    let fixture_dir = format!(
        "{}/../../tests/fixtures/replays",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut frames: Vec<(String, DynamicImage)> = Vec::new();
    for entry in std::fs::read_dir(&fixture_dir).expect("fixtures dir") {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("frame_") && name.ends_with(".jpg") {
            let img = image::open(entry.path()).unwrap_or_else(|e| panic!("open {name}: {e}"));
            frames.push((name, img));
        }
    }
    frames.sort_by(|a, b| a.0.cmp(&b.0));
    frames
}

fn parse_stat(s: &str) -> Option<u32> {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse().ok()
}

fn stat_match(ocr_val: &str, expected: u32) -> bool {
    parse_stat(ocr_val) == Some(expected)
}

#[test]
#[ignore = "requires local OW replay screenshots in tests/fixtures/replays/ (not committed)"]
fn debug_header_offset() {
    for (file, label) in [
        ("replay_01.png", "R01"),
        ("replay_02.png", "R02"),
        ("replay_03.png", "R03"),
        ("replay_04.png", "R04"),
        ("replay_05.png", "R05"),
        ("frame_0001.jpg", "F01"),
        ("frame_0096.jpg", "F96"),
        ("frame_0275.jpg", "F275"),
    ] {
        let img = load_image(file);
        let cropped = preprocess::crop_scoreboard(&img);
        let header_off = preprocess::detect_column_offset(&cropped);

        // Also run a quick OCR probe at the header offset vs 0.0
        let ts = detect_team_size(&cropped);
        let cols_header = preprocess::columns_with_offset(header_off);
        let cols_zero = preprocess::columns_with_offset(0.0);
        let probe = preprocess::crop_player_row(&cropped, 0, ts);
        let (score_h, score_z) = if let Some(ref row) = probe {
            let sh: i32 = (0..6)
                .filter_map(|c| {
                    preprocess::crop_stat_cell(row, c, &cols_header)
                        .and_then(|cell| ocr::recognize_cell(&cell).ok())
                        .filter(|r| {
                            !r.value.is_empty()
                                && r.value.chars().all(|ch| ch.is_ascii_digit() || ch == ',')
                        })
                })
                .count() as i32;
            let sz: i32 = (0..6)
                .filter_map(|c| {
                    preprocess::crop_stat_cell(row, c, &cols_zero)
                        .and_then(|cell| ocr::recognize_cell(&cell).ok())
                        .filter(|r| {
                            !r.value.is_empty()
                                && r.value.chars().all(|ch| ch.is_ascii_digit() || ch == ',')
                        })
                })
                .count() as i32;
            (sh, sz)
        } else {
            (0, 0)
        };

        println!(
            "[{label}] header={header_off:+.4}, score@header={score_h}/6, score@zero={score_z}/6, ts={ts}"
        );
    }
}

#[test]
#[ignore = "requires local OW replay screenshots in tests/fixtures/replays/ (not committed)"]
fn debug_measure_columns() {
    let img = load_image("replay_05.png");
    let cropped = preprocess::crop_scoreboard(&img);
    let binary = preprocess::prepare(&cropped);

    let (w, h) = (binary.width(), binary.height());
    println!("Scoreboard binary: {w}x{h}");

    // Scan header row (first 25 pixels) for white text clusters
    let header_rows = 25u32;
    let mut col_density = vec![0u32; w as usize];
    for y in 0..header_rows.min(h) {
        for x in 0..w {
            // In the Sauvola binary, text is BLACK (0) on white (255).
            // So black pixels = text.
            let px = binary.get_pixel(x, y).0[0];
            if px < 128 {
                col_density[x as usize] += 1;
            }
        }
    }

    // Find clusters of high density (header labels)
    let threshold = header_rows / 3;
    let mut in_cluster = false;
    let mut cluster_start = 0usize;
    let mut clusters = Vec::new();
    for (x, &count) in col_density.iter().enumerate() {
        if count >= threshold {
            if !in_cluster {
                cluster_start = x;
                in_cluster = true;
            }
        } else if in_cluster {
            let center = (cluster_start + x) / 2;
            let width = x - cluster_start;
            clusters.push((cluster_start, x, center, width));
            in_cluster = false;
        }
    }
    if in_cluster {
        let x = w as usize;
        clusters.push((cluster_start, x, (cluster_start + x) / 2, x - cluster_start));
    }

    println!("\nHeader text clusters (first {header_rows}px):");
    for (i, (start, end, center, width)) in clusters.iter().enumerate() {
        let center_ratio = *center as f64 / w as f64;
        let start_ratio = *start as f64 / w as f64;
        println!(
            "  Cluster {i}: x={start}-{end} (center={center}, {center_ratio:.3}), width={width}px, start_ratio={start_ratio:.3}"
        );
    }

    // Scan horizontal line at y=10 (inside header labels area)
    println!("\nHorizontal scan at y=10 (header label area):");
    let mut line_clusters = Vec::new();
    let mut in_cl = false;
    let mut cl_start = 0;
    for x in 0..w {
        let px = binary.get_pixel(x, 10.min(h - 1)).0[0];
        if px < 128 {
            if !in_cl {
                cl_start = x;
                in_cl = true;
            }
        } else if in_cl {
            let center = (cl_start + x) / 2;
            let ratio = center as f64 / w as f64;
            line_clusters.push((cl_start, x, center, ratio));
            in_cl = false;
        }
    }
    for (s, e, c, r) in &line_clusters {
        println!("  x={s}-{e} center={c} ratio={r:.3} width={}", e - s);
    }

    // Scan RGB original to find column header positions
    let rgb = cropped.to_rgb8();
    println!("\nHorizontal brightness at y=8 (raw RGB, looking for bright text on dark):");
    let mut bright_clusters = Vec::new();
    let mut in_br = false;
    let mut br_start = 0u32;
    for x in 0..w {
        let px = rgb.get_pixel(x, 8.min(h - 1));
        let brightness = (px.0[0] as u32 + px.0[1] as u32 + px.0[2] as u32) / 3;
        if brightness > 160 {
            if !in_br {
                br_start = x;
                in_br = true;
            }
        } else if in_br {
            if x - br_start >= 5 {
                let center = (br_start + x) / 2;
                let ratio = center as f64 / w as f64;
                bright_clusters.push((br_start, x, center, ratio));
            }
            in_br = false;
        }
    }
    println!("  Bright text clusters (brightness>160, min 5px wide):");
    for (s, e, c, r) in &bright_clusters {
        println!("    x={s}-{e} center={c} ratio={r:.3} width={}", e - s);
    }

    // Scan a data row (BUGG at y≈100 based on visual inspection)
    // Try multiple y positions to find actual rows
    for scan_y in [40u32, 60, 80, 100, 120, 140] {
        if scan_y >= h {
            continue;
        }
        let mut data_clusters = Vec::new();
        let mut in_cl = false;
        let mut cl_start = 0u32;
        for x in (w / 3)..w {
            let px = rgb.get_pixel(x, scan_y);
            let brightness = (px.0[0] as u32 + px.0[1] as u32 + px.0[2] as u32) / 3;
            if brightness > 160 {
                if !in_cl {
                    cl_start = x;
                    in_cl = true;
                }
            } else if in_cl {
                if x - cl_start >= 3 {
                    let center = (cl_start + x) / 2;
                    let ratio = center as f64 / w as f64;
                    data_clusters.push((cl_start, x, center, ratio, x - cl_start));
                }
                in_cl = false;
            }
        }
        if !data_clusters.is_empty() {
            println!("\nData row scan at y={scan_y} (right half, bright clusters):");
            for (s, e, c, r, width) in &data_clusters {
                println!("  x={s}-{e} center={c} ratio={r:.3} w={width}");
            }
        }
    }

    // Also scan in the HSV-masked image for cleaner text positions
    let masked = preprocess::prepare(&cropped);
    println!("\nBinary image row scans (text = dark pixels < 128):");
    for scan_y in [40u32, 60, 80, 100, 120] {
        if scan_y >= h {
            continue;
        }
        let mut text_clusters = Vec::new();
        let mut in_cl = false;
        let mut cl_start = 0u32;
        for x in (w / 3)..w {
            let px = masked.get_pixel(x, scan_y).0[0];
            if px < 128 {
                if !in_cl {
                    cl_start = x;
                    in_cl = true;
                }
            } else if in_cl {
                if x - cl_start >= 3 {
                    let center = (cl_start + x) / 2;
                    let ratio = center as f64 / w as f64;
                    text_clusters.push((cl_start, x, center, ratio, x - cl_start));
                }
                in_cl = false;
            }
        }
        if !text_clusters.is_empty() {
            println!("  y={scan_y}: {} clusters", text_clusters.len());
            for (s, e, c, r, width) in &text_clusters {
                println!("    x={s}-{e} center={c} ratio={r:.3} w={width}");
            }
        }
    }
}

#[test]
#[ignore = "requires local OW replay screenshots in tests/fixtures/replays/ (not committed)"]
fn debug_save_cell_crops() {
    for (file, label, ts) in [
        ("replay_05.png", "orig", 6usize),
        ("frame_0275.jpg", "new", 5),
    ] {
        let img = load_image(file);
        let cropped = preprocess::crop_scoreboard(&img);
        let debug_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../tests/fixtures/debug_crops_{label}"));
        let _ = std::fs::create_dir_all(&debug_dir);

        println!(
            "[{label}] Scoreboard crop: {}x{}",
            cropped.width(),
            cropped.height()
        );

        let columns = preprocess::columns_with_offset(0.0);
        println!("[{label}] Columns: {:?}", columns);
        for row_idx in 0..(ts * 2) {
            if let Some(row) = preprocess::crop_player_row(&cropped, row_idx, ts) {
                let _ = row.save(debug_dir.join(format!("row_{row_idx:02}.png")));

                let name = preprocess::crop_name_cell(&row);
                let _ = name.save(debug_dir.join(format!("row_{row_idx:02}_name.png")));
                let name_pp = preprocess::prepare_name_cell(&name);
                let _ = DynamicImage::ImageLuma8(name_pp)
                    .save(debug_dir.join(format!("row_{row_idx:02}_name_pp.png")));

                for col in 0..6 {
                    if let Some(cell) = preprocess::crop_stat_cell(&row, col, &columns) {
                        let _ = cell.save(debug_dir.join(format!("row_{row_idx:02}_col{col}.png")));
                        let cell_pp = preprocess::prepare_cell(&cell);
                        let _ = DynamicImage::ImageLuma8(cell_pp)
                            .save(debug_dir.join(format!("row_{row_idx:02}_col{col}_pp.png")));
                    }
                }
            }
        }
        println!("[{label}] Saved to {}", debug_dir.display());
    }
}

#[test]
#[ignore = "requires local OW replay screenshots in tests/fixtures/replays/ (not committed)"]
fn benchmark_ocr_accuracy() {
    let replays = ground_truth_replays();
    let stat_labels = ["E", "A", "D", "DMG", "H", "MIT"];

    let mut total_cells = 0u32;
    let mut correct_cells = 0u32;
    let mut total_names = 0u32;
    let mut correct_names = 0u32;
    let mut total_confidence = 0i64;
    let mut confidence_count = 0u32;

    for replay in &replays {
        let img = load_image(replay.file);
        let results = ocr::recognize_scoreboard_cells_with_team_size(&img, Some(6));

        let all_gt: Vec<&GroundTruth> = replay.team1.iter().chain(replay.team2.iter()).collect();

        println!(
            "\n=== {} ({} rows detected, {} expected) ===",
            replay.file,
            results.len(),
            all_gt.len()
        );

        for (i, (result, gt)) in results.iter().zip(all_gt.iter()).enumerate() {
            let team = if i < 6 { "T1" } else { "T2" };
            let row_in_team = if i < 6 { i } else { i - 6 };

            // Check name
            let name_ocr = result.name.as_ref().map(|n| n.value.as_str()).unwrap_or("");
            let name_ok = name_ocr.to_uppercase().contains(&gt.name.to_uppercase())
                || gt
                    .name
                    .to_uppercase()
                    .contains(name_ocr.to_uppercase().trim());
            total_names += 1;
            if name_ok {
                correct_names += 1;
            }

            // Check stats
            let mut row_correct = 0;
            let mut row_total = 0;
            let mut stat_details = Vec::new();
            for (col, expected) in gt.stats.iter().enumerate() {
                row_total += 1;
                total_cells += 1;
                if let Some(cell) = result.stats.get(col) {
                    let ok = stat_match(&cell.value, *expected);
                    if ok {
                        row_correct += 1;
                        correct_cells += 1;
                    }
                    stat_details.push(format!(
                        "{}:{}{}(exp {})",
                        stat_labels[col],
                        cell.value,
                        if ok { "✓" } else { "✗" },
                        expected
                    ));
                    total_confidence += cell.confidence as i64;
                    confidence_count += 1;
                } else {
                    stat_details.push(format!("{}:MISS(exp {})", stat_labels[col], expected));
                }
            }

            println!(
                "  [{team}R{row_in_team}] name:{}{} (exp:{}) | {}/{} stats | {}",
                name_ocr,
                if name_ok { "✓" } else { "✗" },
                gt.name,
                row_correct,
                row_total,
                stat_details.join(" ")
            );

            total_confidence += result.mean_confidence as i64;
            confidence_count += 1;
        }

        if results.len() != all_gt.len() {
            println!(
                "  ⚠ Row count mismatch: got {} vs expected {}",
                results.len(),
                all_gt.len()
            );
        }
    }

    let stat_accuracy = if total_cells > 0 {
        correct_cells as f64 / total_cells as f64 * 100.0
    } else {
        0.0
    };
    let name_accuracy = if total_names > 0 {
        correct_names as f64 / total_names as f64 * 100.0
    } else {
        0.0
    };
    let mean_conf = if confidence_count > 0 {
        total_confidence as f64 / confidence_count as f64
    } else {
        0.0
    };

    println!("\n========== SUMMARY ==========");
    println!("Stat cell accuracy: {correct_cells}/{total_cells} ({stat_accuracy:.1}%)");
    println!("Name accuracy:      {correct_names}/{total_names} ({name_accuracy:.1}%)");
    println!("Mean confidence:    {mean_conf:.1}");
    println!("=============================");
}

#[test]
#[ignore = "requires local OW replay screenshots in tests/fixtures/replays/ (not committed)"]
fn evaluate_new_frames() {
    let frames = load_all_frames();
    if frames.is_empty() {
        println!("No frame_*.jpg files found, skipping.");
        return;
    }

    let stat_labels = ["E", "A", "D", "DMG", "H", "MIT"];
    let mut global_conf_sum = 0i64;
    let mut global_conf_count = 0u32;
    let mut global_rows = 0u32;
    let mut global_clean_cells = 0u32;
    let mut global_total_cells = 0u32;
    let mut per_image_summary: Vec<(String, usize, f64, f64)> = Vec::new();

    println!("=== Evaluating {} new frame images ===\n", frames.len());

    for (name, img) in &frames {
        let cropped = preprocess::crop_scoreboard(img);
        let team_size = detect_team_size(&cropped);
        let results = ocr::recognize_scoreboard_cells_with_team_size(img, Some(team_size));

        let mut img_conf_sum = 0i64;
        let mut img_conf_count = 0u32;
        let mut img_clean = 0u32;
        let mut img_total = 0u32;

        println!(
            "--- {name} (team_size={team_size}, rows={}) ---",
            results.len()
        );

        for (i, row) in results.iter().enumerate() {
            let team = if i < team_size { "T1" } else { "T2" };
            let row_in_team = if i < team_size { i } else { i - team_size };
            let name_str = row.name.as_ref().map(|n| n.value.as_str()).unwrap_or("?");

            let mut stat_strs = Vec::new();
            for (col, cell) in row.stats.iter().enumerate() {
                let label = stat_labels.get(col).unwrap_or(&"?");
                let clean = !cell.value.is_empty()
                    && cell.value.chars().all(|c| c.is_ascii_digit() || c == ',');
                if clean {
                    img_clean += 1;
                }
                img_total += 1;
                global_total_cells += 1;
                if clean {
                    global_clean_cells += 1;
                }

                img_conf_sum += cell.confidence as i64;
                img_conf_count += 1;
                global_conf_sum += cell.confidence as i64;
                global_conf_count += 1;

                stat_strs.push(format!("{}:{}({}%)", label, cell.value, cell.confidence));
            }

            println!(
                "  [{team}R{row_in_team}] {name_str} | conf={} | {}",
                row.mean_confidence,
                stat_strs.join(" ")
            );
        }

        global_rows += results.len() as u32;
        let mean_conf = if img_conf_count > 0 {
            img_conf_sum as f64 / img_conf_count as f64
        } else {
            0.0
        };
        let clean_pct = if img_total > 0 {
            img_clean as f64 / img_total as f64 * 100.0
        } else {
            0.0
        };
        per_image_summary.push((name.clone(), results.len(), mean_conf, clean_pct));
        println!(
            "  => mean_conf={mean_conf:.1}, clean_cells={img_clean}/{img_total} ({clean_pct:.1}%)\n"
        );
    }

    let global_mean_conf = if global_conf_count > 0 {
        global_conf_sum as f64 / global_conf_count as f64
    } else {
        0.0
    };
    let global_clean_pct = if global_total_cells > 0 {
        global_clean_cells as f64 / global_total_cells as f64 * 100.0
    } else {
        0.0
    };

    println!("\n========== NEW FRAMES SUMMARY ==========");
    println!("Images processed:   {}", frames.len());
    println!("Total rows:         {global_rows}");
    println!("Mean confidence:    {global_mean_conf:.1}");
    println!(
        "Clean cells:        {global_clean_cells}/{global_total_cells} ({global_clean_pct:.1}%)"
    );
    println!("\nPer-image breakdown:");
    for (name, rows, conf, clean) in &per_image_summary {
        println!("  {name}: {rows} rows, conf={conf:.1}, clean={clean:.1}%");
    }
    println!("=========================================");
}

#[test]
#[ignore = "requires local OW replay screenshots in tests/fixtures/replays/ (not committed)"]
fn evaluate_live_frames() {
    let fixture_dir = format!(
        "{}/../../tests/fixtures/replays",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut frames: Vec<(String, DynamicImage)> = Vec::new();
    for entry in std::fs::read_dir(&fixture_dir).expect("fixtures dir") {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("live_") && name.ends_with(".png") {
            let img = image::open(entry.path()).unwrap_or_else(|e| panic!("open {name}: {e}"));
            frames.push((name, img));
        }
    }
    frames.sort_by(|a, b| a.0.cmp(&b.0));

    if frames.is_empty() {
        println!("No live_*.png files found, skipping.");
        return;
    }

    let stat_labels = ["E", "A", "D", "DMG", "H", "MIT"];
    let mut global_conf_sum = 0i64;
    let mut global_conf_count = 0u32;
    let mut global_clean = 0u32;
    let mut global_total = 0u32;

    println!(
        "=== Evaluating {} live gameplay screenshots ===\n",
        frames.len()
    );

    for (name, img) in &frames {
        let cropped = preprocess::crop_scoreboard(img);
        let team_size = detect_team_size(&cropped);
        let results = ocr::recognize_scoreboard_cells_with_team_size(img, Some(team_size));

        let mut img_clean = 0u32;
        let mut img_total = 0u32;

        println!(
            "--- {name} (team_size={team_size}, rows={}) ---",
            results.len()
        );

        for (i, row) in results.iter().enumerate() {
            let team = if i < team_size { "T1" } else { "T2" };
            let row_in_team = if i < team_size { i } else { i - team_size };
            let name_str = row.name.as_ref().map(|n| n.value.as_str()).unwrap_or("?");

            let mut stat_strs = Vec::new();
            for (col, cell) in row.stats.iter().enumerate() {
                let label = stat_labels.get(col).unwrap_or(&"?");
                let clean = !cell.value.is_empty()
                    && cell.value.chars().all(|c| c.is_ascii_digit() || c == ',');
                if clean {
                    img_clean += 1;
                }
                img_total += 1;
                global_total += 1;
                if clean {
                    global_clean += 1;
                }
                global_conf_sum += cell.confidence as i64;
                global_conf_count += 1;
                stat_strs.push(format!("{}:{}({}%)", label, cell.value, cell.confidence));
            }

            println!(
                "  [{team}R{row_in_team}] {name_str} | conf={} | {}",
                row.mean_confidence,
                stat_strs.join(" ")
            );
        }

        let clean_pct = if img_total > 0 {
            img_clean as f64 / img_total as f64 * 100.0
        } else {
            0.0
        };
        println!("  => clean={img_clean}/{img_total} ({clean_pct:.1}%)");
    }

    let mean_conf = if global_conf_count > 0 {
        global_conf_sum as f64 / global_conf_count as f64
    } else {
        0.0
    };
    let clean_pct = if global_total > 0 {
        global_clean as f64 / global_total as f64 * 100.0
    } else {
        0.0
    };
    println!("\n=== LIVE FRAMES SUMMARY ===");
    let count = frames.len();
    println!(
        "Images: {count}, Clean cells: {global_clean}/{global_total} ({clean_pct:.1}%), Mean conf: {mean_conf:.1}"
    );
    println!("===========================");
}
