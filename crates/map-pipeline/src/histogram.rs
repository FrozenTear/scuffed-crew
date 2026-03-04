/// Build a histogram from weighted samples.
/// Returns (bin_edges, bin_counts) where bin_counts[i] covers [bin_edges[i], bin_edges[i+1]).
pub fn build_histogram(
    samples: &[(f64, f64)], // (value, weight) pairs
    bin_width: f64,
) -> (Vec<f64>, Vec<f64>) {
    if samples.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let min_val = samples.iter().map(|(v, _)| *v).fold(f64::INFINITY, f64::min);
    let max_val = samples.iter().map(|(v, _)| *v).fold(f64::NEG_INFINITY, f64::max);

    let n_bins = ((max_val - min_val) / bin_width).ceil() as usize + 1;
    let mut counts = vec![0.0f64; n_bins];
    let mut edges = Vec::with_capacity(n_bins + 1);

    for i in 0..=n_bins {
        edges.push(min_val + i as f64 * bin_width);
    }

    for &(value, weight) in samples {
        let bin = ((value - min_val) / bin_width).floor() as usize;
        let bin = bin.min(n_bins - 1);
        counts[bin] += weight;
    }

    (edges, counts)
}

/// Apply 1D Gaussian smoothing to a signal.
/// Uses a kernel of radius ceil(3*sigma/bin_width) bins.
pub fn gaussian_smooth(signal: &[f64], sigma: f64, bin_width: f64) -> Vec<f64> {
    if signal.is_empty() {
        return Vec::new();
    }

    let sigma_bins = sigma / bin_width;
    let radius = (3.0 * sigma_bins).ceil() as usize;

    if radius == 0 {
        return signal.to_vec();
    }

    // Build kernel
    let kernel_size = 2 * radius + 1;
    let mut kernel = Vec::with_capacity(kernel_size);
    let mut kernel_sum = 0.0;
    for i in 0..kernel_size {
        let x = i as f64 - radius as f64;
        let val = (-0.5 * (x / sigma_bins).powi(2)).exp();
        kernel.push(val);
        kernel_sum += val;
    }
    // Normalize
    for k in &mut kernel {
        *k /= kernel_sum;
    }

    // Convolve
    let n = signal.len();
    let mut result = vec![0.0; n];
    for i in 0..n {
        let mut sum = 0.0;
        for (j, &k) in kernel.iter().enumerate() {
            let idx = i as isize + j as isize - radius as isize;
            let idx = idx.clamp(0, n as isize - 1) as usize;
            sum += signal[idx] * k;
        }
        result[i] = sum;
    }

    result
}

/// Find valleys (local minima) between peaks.
/// Given peak positions (as indices into the signal), find the minimum value
/// between each consecutive pair of peaks. Returns (index, value) pairs.
pub fn find_valleys(signal: &[f64], peak_indices: &[usize]) -> Vec<(usize, f64)> {
    if peak_indices.len() < 2 {
        return Vec::new();
    }

    let mut valleys = Vec::with_capacity(peak_indices.len() - 1);
    for window in peak_indices.windows(2) {
        let (start, end) = (window[0], window[1]);
        let mut min_idx = start;
        let mut min_val = f64::INFINITY;
        for i in start..=end {
            if signal[i] < min_val {
                min_val = signal[i];
                min_idx = i;
            }
        }
        valleys.push((min_idx, min_val));
    }

    valleys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_basic() {
        // 10 samples clustered at 0.0 and 3.0
        let samples: Vec<(f64, f64)> = vec![
            (0.0, 1.0), (0.1, 1.0), (0.2, 1.0), (0.1, 1.0), (0.15, 1.0),
            (3.0, 1.0), (3.1, 1.0), (3.2, 1.0), (3.05, 1.0), (3.15, 1.0),
        ];
        let (edges, counts) = build_histogram(&samples, 0.5);
        // Should have bins from 0.0 to ~3.5
        assert!(edges.len() >= 2);
        assert_eq!(counts.len(), edges.len() - 1);
        // First bin (0.0-0.5) should have weight 5.0
        assert_eq!(counts[0], 5.0);
        // Bin containing 3.0-3.5 should have weight 5.0
        let bin_3 = ((3.0 - edges[0]) / 0.5).floor() as usize;
        assert_eq!(counts[bin_3], 5.0);
    }

    #[test]
    fn histogram_empty() {
        let (edges, counts) = build_histogram(&[], 0.5);
        assert!(edges.is_empty());
        assert!(counts.is_empty());
    }

    #[test]
    fn histogram_weighted() {
        // One sample with weight 10 should produce a bin with count 10
        let samples = vec![(1.0, 10.0)];
        let (_, counts) = build_histogram(&samples, 0.5);
        let bin = counts.iter().find(|&&c| c > 0.0).unwrap();
        assert_eq!(*bin, 10.0);
    }

    #[test]
    fn gaussian_smooth_identity() {
        // With sigma=0, smoothing should be identity (or near-identity)
        let signal = vec![0.0, 0.0, 10.0, 0.0, 0.0];
        let smoothed = gaussian_smooth(&signal, 0.001, 1.0);
        assert_eq!(smoothed.len(), 5);
        // Peak should still be at index 2
        let max_idx = smoothed.iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap().0;
        assert_eq!(max_idx, 2);
    }

    #[test]
    fn gaussian_smooth_spreads_peak() {
        // With larger sigma, the peak should spread
        let signal = vec![0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0];
        let smoothed = gaussian_smooth(&signal, 1.0, 1.0);
        // Neighbors should now have nonzero values
        assert!(smoothed[2] > 0.0);
        assert!(smoothed[4] > 0.0);
        // Peak should still be at center
        let max_idx = smoothed.iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap().0;
        assert_eq!(max_idx, 3);
    }

    #[test]
    fn gaussian_smooth_empty() {
        let smoothed = gaussian_smooth(&[], 1.0, 1.0);
        assert!(smoothed.is_empty());
    }

    #[test]
    fn find_valleys_between_two_peaks() {
        // Signal with two peaks at indices 2 and 7, valley around index 5
        let signal = vec![0.0, 5.0, 10.0, 5.0, 2.0, 1.0, 2.0, 8.0, 3.0];
        let valleys = find_valleys(&signal, &[2, 7]);
        assert_eq!(valleys.len(), 1);
        assert_eq!(valleys[0].0, 5); // valley at index 5 (value 1.0)
        assert_eq!(valleys[0].1, 1.0);
    }

    #[test]
    fn find_valleys_single_peak() {
        let signal = vec![0.0, 5.0, 10.0, 5.0, 0.0];
        let valleys = find_valleys(&signal, &[2]);
        assert!(valleys.is_empty()); // No valleys with only one peak
    }
}
