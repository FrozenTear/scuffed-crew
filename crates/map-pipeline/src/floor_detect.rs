use crate::config::{DetectionConfig, FloorConfig};
use crate::histogram::{build_histogram, find_valleys, gaussian_smooth};
use crate::mesh::Triangle;

/// Result of floor detection: a list of detected floor levels.
pub struct FloorDetectionResult {
    pub floors: Vec<FloorConfig>,
    /// The smoothed histogram (for diagnostic display).
    pub histogram_edges: Vec<f64>,
    pub histogram_values: Vec<f64>,
    /// Indices of detected peaks in the histogram.
    pub peak_indices: Vec<usize>,
}

/// Detect floor levels from mesh triangles.
pub fn detect_floors(
    triangles: &[Triangle],
    config: &DetectionConfig,
) -> anyhow::Result<FloorDetectionResult> {
    // 1. Filter to walkable faces and collect (Y-centroid, area) pairs
    let walkable_samples: Vec<(f64, f64)> = triangles
        .iter()
        .filter(|t| t.is_walkable(config.walkable_slope_max_degrees))
        .filter(|t| t.area() > 0.001) // Skip degenerate triangles
        .map(|t| (t.centroid_y() as f64, t.area() as f64))
        .collect();

    if walkable_samples.is_empty() {
        anyhow::bail!(
            "No walkable surfaces found. Check walkable_slope_max_degrees ({} deg).",
            config.walkable_slope_max_degrees
        );
    }

    tracing::info!("Found {} walkable faces", walkable_samples.len());

    // 2. Build histogram
    let (edges, counts) = build_histogram(&walkable_samples, config.histogram_bin_width);

    // 3. Gaussian smooth
    let smoothed = gaussian_smooth(&counts, config.gaussian_sigma, config.histogram_bin_width);

    // 4. Peak detection
    // Scale prominence relative to the histogram's max value so it works
    // with both small test data and real maps with hundreds of thousands of faces.
    let max_signal = smoothed.iter().cloned().fold(0.0f64, f64::max);
    let prominence = if max_signal > 100.0 {
        max_signal * (config.peak_min_prominence / 100.0)
    } else {
        config.peak_min_prominence
    };
    let min_distance = (config.min_floor_gap / config.histogram_bin_width).ceil() as usize;
    let peaks = find_peaks_in_signal(&smoothed, prominence, min_distance);
    tracing::info!(
        "Peak detection: max_signal={:.1}, prominence_threshold={:.1}",
        max_signal,
        prominence
    );

    if peaks.is_empty() {
        anyhow::bail!(
            "No floor peaks detected. Try lowering peak_min_prominence (currently {}).",
            config.peak_min_prominence
        );
    }

    tracing::info!("Detected {} floor peaks", peaks.len());

    // 5. Find valleys between peaks → floor boundaries
    let valleys = find_valleys(&smoothed, &peaks);

    // 6. Convert to FloorConfig
    let y_min_global = edges.first().copied().unwrap_or(0.0);
    let y_max_global = edges.last().copied().unwrap_or(0.0);

    let mut floors = Vec::new();
    for (i, &peak_idx) in peaks.iter().enumerate() {
        let floor_y_min = if i == 0 {
            y_min_global
        } else {
            let valley_idx = valleys[i - 1].0;
            edges[valley_idx]
        };

        let floor_y_max = if i == peaks.len() - 1 {
            y_max_global
        } else {
            let valley_idx = valleys[i].0;
            edges[valley_idx]
        };

        let peak_y = edges[peak_idx] + config.histogram_bin_width / 2.0;
        let name = format!("Floor {}", i + 1);
        let id = format!("floor_{}", i + 1);

        tracing::info!(
            "  {} (peak Y={:.1}m): [{:.1}m, {:.1}m]",
            name,
            peak_y,
            floor_y_min,
            floor_y_max
        );

        floors.push(FloorConfig {
            id,
            name,
            y_min: floor_y_min,
            y_max: floor_y_max,
            is_default: i == 0, // First floor is default (lowest)
        });
    }

    // Make the floor closest to y=0 the default (most likely "ground")
    if let Some(ground_idx) = floors
        .iter()
        .enumerate()
        .min_by_key(|(_, f)| {
            let mid = (f.y_min + f.y_max) / 2.0;
            (mid.abs() * 1000.0) as i64
        })
        .map(|(i, _)| i)
    {
        for (i, floor) in floors.iter_mut().enumerate() {
            floor.is_default = i == ground_idx;
        }
    }

    Ok(FloorDetectionResult {
        floors,
        histogram_edges: edges,
        histogram_values: smoothed,
        peak_indices: peaks,
    })
}

/// Wrapper around `find_peaks` crate.
fn find_peaks_in_signal(signal: &[f64], min_prominence: f64, min_distance: usize) -> Vec<usize> {
    use find_peaks::PeakFinder;

    let mut finder = PeakFinder::new(signal);
    finder.with_min_prominence(min_prominence);
    finder.with_min_distance(min_distance);

    let peaks = finder.find_peaks();
    let mut indices: Vec<usize> = peaks.into_iter().map(|p| p.middle_position()).collect();
    indices.sort_unstable();
    indices
}

/// Print a simple ASCII histogram to the terminal for diagnostics.
pub fn print_histogram(result: &FloorDetectionResult) {
    let max_val = result
        .histogram_values
        .iter()
        .cloned()
        .fold(0.0f64, f64::max);
    if max_val == 0.0 {
        return;
    }

    let bar_width = 60;
    println!("\n  Y (m)  | Walkable surface area");
    println!("  -------+{}", "-".repeat(bar_width + 2));

    for (i, &val) in result.histogram_values.iter().enumerate() {
        let y = result.histogram_edges[i];
        let bar_len = ((val / max_val) * bar_width as f64).round() as usize;
        let bar: String = "#".repeat(bar_len);
        let marker = if result.peak_indices.contains(&i) {
            " <-- FLOOR"
        } else {
            ""
        };
        println!("  {:6.1} | {}{}", y, bar, marker);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn make_floor_patch(y: f32, count: usize, area: f32) -> Vec<Triangle> {
        // Create `count` flat horizontal triangles centered at height `y`.
        // Uses a triangular distribution (concentrated at center) across ±1.5m
        // so the histogram has a clear peak for detection.
        // Winding order produces upward (+Y) normal.
        let scale = (area * 2.0).sqrt();
        (0..count)
            .map(|i| {
                let x = i as f32 * scale;
                // Triangular distribution: more samples near center
                let t = i as f32 / count as f32; // 0..1
                let t_centered = (t - 0.5) * 2.0; // -1..1
                let y_offset = t_centered * t_centered.abs() * 1.5; // cubic-ish, ±1.5m, concentrated at 0
                Triangle::new(
                    Vec3::new(x, y + y_offset, 0.0),
                    Vec3::new(x, y + y_offset, scale),
                    Vec3::new(x + scale, y + y_offset, 0.0),
                )
            })
            .collect()
    }

    #[test]
    fn detect_two_floors() {
        let config = DetectionConfig {
            walkable_slope_max_degrees: 50.0,
            histogram_bin_width: 0.25,
            gaussian_sigma: 0.4,
            min_floor_gap: 2.0,
            peak_min_prominence: 1.0, // Lower for test data
        };

        // Ground floor at y=0, upper floor at y=4
        let mut triangles = make_floor_patch(0.0, 50, 1.0);
        triangles.extend(make_floor_patch(4.0, 50, 1.0));

        let result = detect_floors(&triangles, &config).unwrap();
        assert_eq!(result.floors.len(), 2);
        // First floor should contain y=0, second should contain y=4
        assert!(result.floors[0].y_min <= 0.0 && result.floors[0].y_max >= 0.0);
        assert!(result.floors[1].y_min <= 4.0 && result.floors[1].y_max >= 4.0);
    }

    #[test]
    fn detect_single_floor() {
        let config = DetectionConfig {
            walkable_slope_max_degrees: 50.0,
            histogram_bin_width: 0.25,
            gaussian_sigma: 0.4,
            min_floor_gap: 2.0,
            peak_min_prominence: 1.0,
        };

        let triangles = make_floor_patch(0.0, 100, 1.0);

        let result = detect_floors(&triangles, &config).unwrap();
        assert_eq!(result.floors.len(), 1);
        assert!(result.floors[0].is_default);
    }

    #[test]
    fn detect_no_walkable_faces() {
        let config = DetectionConfig::default();

        // All vertical walls — no walkable surfaces
        let triangles = vec![Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::new(0.0, 0.0, 5.0),
        )];

        let result = detect_floors(&triangles, &config);
        assert!(result.is_err());
    }

    #[test]
    fn ground_floor_is_default() {
        let config = DetectionConfig {
            walkable_slope_max_degrees: 50.0,
            histogram_bin_width: 0.25,
            gaussian_sigma: 0.4,
            min_floor_gap: 2.0,
            peak_min_prominence: 1.0,
        };

        // Three floors: underground (-5), ground (0), upper (5)
        let mut triangles = make_floor_patch(-5.0, 50, 1.0);
        triangles.extend(make_floor_patch(0.0, 50, 1.0));
        triangles.extend(make_floor_patch(5.0, 50, 1.0));

        let result = detect_floors(&triangles, &config).unwrap();
        // Floor closest to y=0 should be default
        let default_floor = result.floors.iter().find(|f| f.is_default).unwrap();
        let default_mid = (default_floor.y_min + default_floor.y_max) / 2.0;
        assert!(
            default_mid.abs() < 2.0,
            "Default floor mid={} should be near y=0",
            default_mid
        );
    }
}
