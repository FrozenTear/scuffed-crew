//! Pre-compute scene XZ bounds and mesh instances from GLB without Bevy.
//! Uses the same gltf crate as map-pipeline.

use glam::{Mat4, Vec3};
use std::path::Path;

/// Axis-aligned bounding box in XZ plane (Bevy coordinates: Y is up).
pub struct SceneBounds {
    pub x_min: f32,
    pub x_max: f32,
    pub z_min: f32,
    pub z_max: f32,
}

impl SceneBounds {
    pub fn width(&self) -> f32 {
        self.x_max - self.x_min
    }
    pub fn height(&self) -> f32 {
        self.z_max - self.z_min
    }
    pub fn center_x(&self) -> f32 {
        (self.x_min + self.x_max) / 2.0
    }
    pub fn center_z(&self) -> f32 {
        (self.z_min + self.z_max) / 2.0
    }
}

/// A mesh placed in world space, identified by its glTF mesh index.
#[derive(Clone)]
pub struct MeshInstance {
    pub mesh_index: usize,
    pub world_transform: Mat4,
}

/// Combined scene analysis result.
pub struct SceneInfo {
    pub bounds: SceneBounds,
    pub mesh_instances: Vec<MeshInstance>,
}

/// Analyze GLB: collect mesh instances and compute bounds.
/// `max_distance` filters out geometry far from the scene centroid (in XZ plane).
pub fn analyze_glb(path: &Path, max_distance: f32) -> anyhow::Result<SceneInfo> {
    let (doc, buffers, _) = gltf::import(path)?;

    let mut instances = Vec::new();

    // Pass 1: collect mesh instances and compute centroid of mesh origins
    let mut centroid_sum = Vec3::ZERO;
    let mut centroid_count = 0u32;

    for scene in doc.scenes() {
        for node in scene.nodes() {
            collect_instances(&node, Mat4::IDENTITY, &mut instances);
        }
    }

    for inst in &instances {
        let origin = inst.world_transform.transform_point3(Vec3::ZERO);
        centroid_sum += origin;
        centroid_count += 1;
    }

    if centroid_count == 0 {
        anyhow::bail!("No mesh geometry found in GLB");
    }

    let centroid = centroid_sum / centroid_count as f32;
    let max_dist_sq = max_distance * max_distance;

    // Pass 2: compute bounds only for vertices within max_distance of centroid
    let mut acc = BoundsAccumulator::default();

    for scene in doc.scenes() {
        for node in scene.nodes() {
            compute_bounded(
                &node,
                Mat4::IDENTITY,
                &buffers,
                centroid,
                max_dist_sq,
                &mut acc,
            );
        }
    }

    if !acc.found_any {
        anyhow::bail!("No mesh geometry within {max_distance}m of scene centroid");
    }

    eprintln!(
        "Scene centroid: ({:.1}, {:.1}), {} meshes, bounds filtered to {:.0}m radius",
        centroid.x, centroid.z, centroid_count, max_distance
    );

    Ok(SceneInfo {
        bounds: SceneBounds {
            x_min: acc.x_min,
            x_max: acc.x_max,
            z_min: acc.z_min,
            z_max: acc.z_max,
        },
        mesh_instances: instances,
    })
}

fn collect_instances(node: &gltf::Node, parent_transform: Mat4, instances: &mut Vec<MeshInstance>) {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent_transform * local;

    if let Some(mesh) = node.mesh() {
        instances.push(MeshInstance {
            mesh_index: mesh.index(),
            world_transform: world,
        });
    }

    for child in node.children() {
        collect_instances(&child, world, instances);
    }
}

fn compute_bounded(
    node: &gltf::Node,
    parent_transform: Mat4,
    buffers: &[gltf::buffer::Data],
    centroid: Vec3,
    max_dist_sq: f32,
    acc: &mut BoundsAccumulator,
) {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent_transform * local;

    if let Some(mesh) = node.mesh() {
        // Check if mesh origin is within radius
        let origin = world.transform_point3(Vec3::ZERO);
        let dx = origin.x - centroid.x;
        let dz = origin.z - centroid.z;
        if dx * dx + dz * dz <= max_dist_sq {
            for primitive in mesh.primitives() {
                let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
                if let Some(positions) = reader.read_positions() {
                    for pos in positions {
                        acc.update(world.transform_point3(Vec3::from(pos)));
                    }
                }
            }
        }
    }

    for child in node.children() {
        compute_bounded(&child, world, buffers, centroid, max_dist_sq, acc);
    }
}

struct BoundsAccumulator {
    x_min: f32,
    x_max: f32,
    z_min: f32,
    z_max: f32,
    found_any: bool,
}

impl BoundsAccumulator {
    fn update(&mut self, pos: Vec3) {
        self.x_min = self.x_min.min(pos.x);
        self.x_max = self.x_max.max(pos.x);
        self.z_min = self.z_min.min(pos.z);
        self.z_max = self.z_max.max(pos.z);
        self.found_any = true;
    }
}

impl Default for BoundsAccumulator {
    fn default() -> Self {
        Self {
            x_min: f32::INFINITY,
            x_max: f32::NEG_INFINITY,
            z_min: f32::INFINITY,
            z_max: f32::NEG_INFINITY,
            found_any: false,
        }
    }
}
