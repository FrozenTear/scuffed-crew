use glam::{Mat4, Vec3};

/// A triangle in world space with precomputed normal.
#[derive(Debug, Clone)]
pub struct Triangle {
    pub v0: Vec3,
    pub v1: Vec3,
    pub v2: Vec3,
    pub normal: Vec3,
}

impl Triangle {
    pub fn new(v0: Vec3, v1: Vec3, v2: Vec3) -> Self {
        let edge1 = v1 - v0;
        let edge2 = v2 - v0;
        let normal = edge1.cross(edge2).normalize_or_zero();
        Self { v0, v1, v2, normal }
    }

    /// Area of the triangle.
    pub fn area(&self) -> f32 {
        let edge1 = self.v1 - self.v0;
        let edge2 = self.v2 - self.v0;
        edge1.cross(edge2).length() * 0.5
    }

    /// Y-coordinate of the centroid (for floor detection).
    pub fn centroid_y(&self) -> f32 {
        (self.v0.y + self.v1.y + self.v2.y) / 3.0
    }

    /// Whether this face is walkable (normal points mostly upward).
    /// Uses dot product with UP vector; threshold is cos(max_slope_degrees).
    pub fn is_walkable(&self, max_slope_degrees: f64) -> bool {
        let threshold = (max_slope_degrees.to_radians()).cos() as f32;
        self.normal.dot(Vec3::Y) > threshold
    }
}

/// Load all mesh triangles from a glTF file, transformed to world space.
pub fn load_glb(path: &std::path::Path) -> anyhow::Result<Vec<Triangle>> {
    let (document, buffers, _images) = gltf::import(path)?;

    let mut triangles = Vec::new();

    for scene in document.scenes() {
        for node in scene.nodes() {
            collect_node_triangles(&node, Mat4::IDENTITY, &buffers, &mut triangles);
        }
    }

    tracing::info!("Loaded {} triangles from {:?}", triangles.len(), path);
    Ok(triangles)
}

fn collect_node_triangles(
    node: &gltf::Node,
    parent_transform: Mat4,
    buffers: &[gltf::buffer::Data],
    triangles: &mut Vec<Triangle>,
) {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent_transform * local;

    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

            let positions: Vec<Vec3> = match reader.read_positions() {
                Some(pos) => pos.map(|p| world.transform_point3(Vec3::from(p))).collect(),
                None => continue,
            };

            if let Some(indices) = reader.read_indices() {
                let indices: Vec<u32> = indices.into_u32().collect();
                for tri in indices.chunks_exact(3) {
                    let v0 = positions[tri[0] as usize];
                    let v1 = positions[tri[1] as usize];
                    let v2 = positions[tri[2] as usize];
                    triangles.push(Triangle::new(v0, v1, v2));
                }
            } else {
                // Non-indexed geometry
                for tri in positions.chunks_exact(3) {
                    triangles.push(Triangle::new(tri[0], tri[1], tri[2]));
                }
            }
        }
    }

    for child in node.children() {
        collect_node_triangles(&child, world, buffers, triangles);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_flat_floor_is_walkable() {
        // Flat horizontal triangle (normal pointing straight up)
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
        );
        assert!(
            (tri.normal.y - 1.0).abs() < 0.01,
            "Normal should be (0,1,0), got {:?}",
            tri.normal
        );
        assert!(tri.is_walkable(50.0));
    }

    #[test]
    fn triangle_wall_not_walkable() {
        // Vertical wall (normal pointing along X)
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        );
        assert!(
            tri.normal.y.abs() < 0.01,
            "Wall normal Y should be ~0, got {}",
            tri.normal.y
        );
        assert!(!tri.is_walkable(50.0));
    }

    #[test]
    fn triangle_steep_slope_not_walkable() {
        // 60-degree slope (normal 30 degrees from horizontal)
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 1.732, 0.0), // tan(60°) = 1.732
            Vec3::new(0.0, 0.0, 1.0),
        );
        assert!(!tri.is_walkable(50.0)); // 60° > 50° max
    }

    #[test]
    fn triangle_area() {
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 2.0),
        );
        assert!((tri.area() - 2.0).abs() < 0.01); // 2x2 right triangle = area 2
    }

    #[test]
    fn triangle_centroid_y() {
        let tri = Triangle::new(
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(0.0, 3.0, 1.0),
        );
        assert!((tri.centroid_y() - 2.0).abs() < 0.01); // (1+2+3)/3 = 2
    }

    #[test]
    fn degenerate_triangle_zero_area() {
        // Degenerate (collinear points)
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
        );
        assert!(tri.area() < 0.001);
        assert!(!tri.is_walkable(50.0)); // Zero normal → not walkable
    }
}
