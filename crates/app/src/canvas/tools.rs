//! Hit testing and geometry utilities
//!
//! Pure functions for point-in-shape testing, path simplification,
//! and spline interpolation used by the canvas editor.

use scuffed_types::Position;

/// Check if a point is within a circle
pub fn point_in_circle(point: &Position, center: &Position, radius: f64) -> bool {
    point.distance_to(center) <= radius
}

/// Check if a point is on a line segment (with tolerance)
pub fn point_on_line(point: &Position, start: &Position, end: &Position, tolerance: f64) -> bool {
    let line_len = start.distance_to(end);
    if line_len == 0.0 {
        return point.distance_to(start) <= tolerance;
    }

    let t = ((point.x - start.x) * (end.x - start.x) + (point.y - start.y) * (end.y - start.y))
        / (line_len * line_len);

    let t = t.clamp(0.0, 1.0);

    let closest = Position::new(
        start.x + t * (end.x - start.x),
        start.y + t * (end.y - start.y),
    );

    point.distance_to(&closest) <= tolerance
}

/// Check if a point is on a path (series of line segments)
pub fn point_on_path(point: &Position, path: &[Position], tolerance: f64) -> bool {
    for i in 0..path.len().saturating_sub(1) {
        if point_on_line(point, &path[i], &path[i + 1], tolerance) {
            return true;
        }
    }
    false
}

/// Check if a point is inside a polygon (ray casting algorithm)
pub fn point_in_polygon(point: &Position, polygon: &[Position]) -> bool {
    if polygon.len() < 3 {
        return false;
    }

    let mut inside = false;
    let mut j = polygon.len() - 1;

    for i in 0..polygon.len() {
        let pi = &polygon[i];
        let pj = &polygon[j];

        if ((pi.y > point.y) != (pj.y > point.y))
            && (point.x < (pj.x - pi.x) * (point.y - pi.y) / (pj.y - pi.y) + pi.x)
        {
            inside = !inside;
        }
        j = i;
    }

    inside
}

/// Calculate bounding box for a set of points
///
/// Returns `Some((min_corner, max_corner))` or `None` if the slice is empty.
pub fn bounding_box(points: &[Position]) -> Option<(Position, Position)> {
    if points.is_empty() {
        return None;
    }

    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;

    for p in points {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }

    Some((Position::new(min_x, min_y), Position::new(max_x, max_y)))
}

/// Calculate bounds as a `scuffed_types::Bounds` struct
pub fn calculate_bounds(points: &[Position]) -> Option<scuffed_types::Bounds> {
    bounding_box(points).map(|(min, max)| {
        scuffed_types::Bounds::new(min.x, min.y, max.x - min.x, max.y - min.y)
    })
}

/// Simplify a path using the Ramer-Douglas-Peucker algorithm
///
/// Reduces the number of points in a path while preserving its overall shape.
/// `epsilon` controls the maximum allowed deviation from the original path.
pub fn simplify_path(points: &[Position], epsilon: f64) -> Vec<Position> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let mut max_dist = 0.0;
    let mut index = 0;

    let start = &points[0];
    let end = &points[points.len() - 1];

    for (i, point) in points.iter().enumerate().skip(1).take(points.len() - 2) {
        let dist = perpendicular_distance(point, start, end);
        if dist > max_dist {
            max_dist = dist;
            index = i;
        }
    }

    if max_dist > epsilon {
        let mut left = simplify_path(&points[..=index], epsilon);
        let right = simplify_path(&points[index..], epsilon);

        left.pop(); // Remove duplicate point
        left.extend(right);
        left
    } else {
        vec![start.clone(), end.clone()]
    }
}

/// Calculate perpendicular distance from a point to a line segment
fn perpendicular_distance(point: &Position, line_start: &Position, line_end: &Position) -> f64 {
    let dx = line_end.x - line_start.x;
    let dy = line_end.y - line_start.y;

    let line_len = (dx * dx + dy * dy).sqrt();

    if line_len == 0.0 {
        return point.distance_to(line_start);
    }

    ((dy * point.x - dx * point.y + line_end.x * line_start.y - line_end.y * line_start.x).abs())
        / line_len
}

/// Smooth a path using Catmull-Rom spline interpolation
///
/// `segments` controls how many interpolated points are generated between each pair
/// of original points. Higher values produce smoother curves.
pub fn smooth_path(points: &[Position], segments: usize) -> Vec<Position> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let mut result = Vec::new();

    for i in 0..points.len() - 1 {
        let p0 = if i == 0 { &points[0] } else { &points[i - 1] };
        let p1 = &points[i];
        let p2 = &points[i + 1];
        let p3 = if i + 2 >= points.len() {
            &points[points.len() - 1]
        } else {
            &points[i + 2]
        };

        for j in 0..segments {
            let t = j as f64 / segments as f64;
            let point = catmull_rom(p0, p1, p2, p3, t);
            result.push(point);
        }
    }

    result.push(points.last().unwrap().clone());
    result
}

/// Catmull-Rom spline interpolation between four control points
fn catmull_rom(p0: &Position, p1: &Position, p2: &Position, p3: &Position, t: f64) -> Position {
    let t2 = t * t;
    let t3 = t2 * t;

    let x = 0.5
        * ((2.0 * p1.x)
            + (-p0.x + p2.x) * t
            + (2.0 * p0.x - 5.0 * p1.x + 4.0 * p2.x - p3.x) * t2
            + (-p0.x + 3.0 * p1.x - 3.0 * p2.x + p3.x) * t3);

    let y = 0.5
        * ((2.0 * p1.y)
            + (-p0.y + p2.y) * t
            + (2.0 * p0.y - 5.0 * p1.y + 4.0 * p2.y - p3.y) * t2
            + (-p0.y + 3.0 * p1.y - 3.0 * p2.y + p3.y) * t3);

    Position::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_in_circle() {
        let center = Position::new(100.0, 100.0);
        assert!(point_in_circle(&Position::new(100.0, 100.0), &center, 10.0));
        assert!(point_in_circle(&Position::new(105.0, 100.0), &center, 10.0));
        assert!(!point_in_circle(
            &Position::new(115.0, 100.0),
            &center,
            10.0
        ));
    }

    #[test]
    fn test_point_on_line() {
        let start = Position::new(0.0, 0.0);
        let end = Position::new(100.0, 0.0);
        assert!(point_on_line(&Position::new(50.0, 0.0), &start, &end, 1.0));
        assert!(point_on_line(&Position::new(50.0, 0.5), &start, &end, 1.0));
        assert!(!point_on_line(
            &Position::new(50.0, 5.0),
            &start,
            &end,
            1.0
        ));
    }

    #[test]
    fn test_point_in_polygon() {
        let polygon = vec![
            Position::new(0.0, 0.0),
            Position::new(100.0, 0.0),
            Position::new(100.0, 100.0),
            Position::new(0.0, 100.0),
        ];

        assert!(point_in_polygon(&Position::new(50.0, 50.0), &polygon));
        assert!(!point_in_polygon(&Position::new(150.0, 50.0), &polygon));
    }

    #[test]
    fn test_bounding_box() {
        let points = vec![
            Position::new(10.0, 20.0),
            Position::new(50.0, 80.0),
            Position::new(30.0, 40.0),
        ];
        let (min, max) = bounding_box(&points).unwrap();
        assert_eq!(min.x, 10.0);
        assert_eq!(min.y, 20.0);
        assert_eq!(max.x, 50.0);
        assert_eq!(max.y, 80.0);
    }

    #[test]
    fn test_bounding_box_empty() {
        assert!(bounding_box(&[]).is_none());
    }

    #[test]
    fn test_simplify_path_short() {
        let points = vec![Position::new(0.0, 0.0), Position::new(10.0, 10.0)];
        let simplified = simplify_path(&points, 1.0);
        assert_eq!(simplified.len(), 2);
    }

    #[test]
    fn test_simplify_path_collinear() {
        // Points on a straight line should simplify to two endpoints
        let points = vec![
            Position::new(0.0, 0.0),
            Position::new(5.0, 0.0),
            Position::new(10.0, 0.0),
        ];
        let simplified = simplify_path(&points, 1.0);
        assert_eq!(simplified.len(), 2);
    }

    #[test]
    fn test_smooth_path_short() {
        let points = vec![Position::new(0.0, 0.0), Position::new(10.0, 10.0)];
        let smoothed = smooth_path(&points, 4);
        assert_eq!(smoothed.len(), 2);
    }

    #[test]
    fn test_calculate_bounds() {
        let points = vec![
            Position::new(10.0, 20.0),
            Position::new(50.0, 80.0),
        ];
        let bounds = calculate_bounds(&points).unwrap();
        assert_eq!(bounds.x, 10.0);
        assert_eq!(bounds.y, 20.0);
        assert_eq!(bounds.width, 40.0);
        assert_eq!(bounds.height, 60.0);
    }
}
