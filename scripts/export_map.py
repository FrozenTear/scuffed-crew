"""
Blender script to export OW maps for the tactical map pipeline.

Usage:
    blender -b map.blend -P scripts/export_map.py -- \
        --config maps/kings_row.toml \
        --output ./out/kings_row/

Requires: Blender 5.0+, io_scene_owm addon installed.
"""

import bpy
import mathutils
import sys
import os
import json
import math
import argparse

# ─────────────────────────────────────────────────────
# Argument parsing (args after --)
# ─────────────────────────────────────────────────────

def parse_args():
    argv = sys.argv
    if "--" in argv:
        argv = argv[argv.index("--") + 1:]
    else:
        argv = []

    parser = argparse.ArgumentParser(description="Export OW map for tactical pipeline")
    parser.add_argument("--config", required=True, help="Path to map TOML config")
    parser.add_argument("--output", required=True, help="Output directory")
    parser.add_argument("--export-glb", action="store_true", help="Also export .glb geometry")
    parser.add_argument("--skip-render", action="store_true", help="Skip rendering floor PNGs")
    return parser.parse_args(argv)


# ─────────────────────────────────────────────────────
# TOML parsing (minimal, no external deps)
# ─────────────────────────────────────────────────────

def parse_toml_simple(path):
    """Minimal TOML parser for our config format. Handles sections, key=value, [[arrays]]."""
    config = {}
    current_section = None
    current_array = None

    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue

            if line.startswith("[[") and line.endswith("]]"):
                key = line[2:-2].strip()
                if key not in config:
                    config[key] = []
                current_section = {}
                config[key].append(current_section)
                current_array = key
                continue

            if line.startswith("[") and line.endswith("]"):
                key = line[1:-1].strip()
                config[key] = {}
                current_section = config[key]
                current_array = None
                continue

            if "=" in line and current_section is not None:
                k, v = line.split("=", 1)
                k = k.strip()
                v = v.strip()
                # Parse value
                if v.startswith('"') and v.endswith('"'):
                    v = v[1:-1]
                elif v == "true":
                    v = True
                elif v == "false":
                    v = False
                else:
                    try:
                        v = float(v) if "." in v else int(v)
                    except ValueError:
                        pass
                current_section[k] = v

    return config


# ─────────────────────────────────────────────────────
# Scene cleanup
# ─────────────────────────────────────────────────────

def cleanup_scene(cleanup_config):
    """Remove non-gameplay objects from the scene."""
    max_dist = cleanup_config.get("max_distance_from_center", 200.0)
    min_size = cleanup_config.get("min_object_size", 0.01)
    skybox_threshold = cleanup_config.get("skybox_size_threshold", 500.0)

    removed = 0

    # Remove lights
    if cleanup_config.get("remove_lights", True):
        for obj in list(bpy.data.objects):
            if obj.type == "LIGHT":
                bpy.data.objects.remove(obj, do_unlink=True)
                removed += 1

        # Remove "Lights" collection if it exists
        if "Lights" in bpy.data.collections:
            col = bpy.data.collections["Lights"]
            for obj in list(col.objects):
                bpy.data.objects.remove(obj, do_unlink=True)
                removed += 1
            bpy.data.collections.remove(col)

    # Remove cameras
    if cleanup_config.get("remove_cameras", True):
        for obj in list(bpy.data.objects):
            if obj.type == "CAMERA":
                bpy.data.objects.remove(obj, do_unlink=True)
                removed += 1

    # Remove particles
    if cleanup_config.get("remove_particles", True):
        for obj in list(bpy.data.objects):
            if obj.particle_systems:
                bpy.data.objects.remove(obj, do_unlink=True)
                removed += 1

    # Remove skybox (objects larger than threshold)
    for obj in list(bpy.data.objects):
        if obj.type != "MESH":
            continue
        dims = obj.dimensions
        if max(dims.x, dims.y, dims.z) > skybox_threshold:
            print(f"  Removing skybox: {obj.name} (dims: {dims.x:.0f}x{dims.y:.0f}x{dims.z:.0f})")
            bpy.data.objects.remove(obj, do_unlink=True)
            removed += 1

    # Remove OOB objects (too far from center)
    for obj in list(bpy.data.objects):
        if obj.type != "MESH":
            continue
        loc = obj.location
        dist = math.sqrt(loc.x**2 + loc.y**2 + loc.z**2)
        if dist > max_dist:
            bpy.data.objects.remove(obj, do_unlink=True)
            removed += 1

    # Remove tiny objects
    for obj in list(bpy.data.objects):
        if obj.type != "MESH":
            continue
        dims = obj.dimensions
        if max(dims.x, dims.y, dims.z) < min_size:
            bpy.data.objects.remove(obj, do_unlink=True)
            removed += 1

    print(f"  Cleanup: removed {removed} objects")


# ─────────────────────────────────────────────────────
# Rendering
# ─────────────────────────────────────────────────────

def get_scene_bounds():
    """Get the XY bounding box of all mesh objects (Blender Z-up)."""
    x_min = y_min = float("inf")
    x_max = y_max = float("-inf")

    for obj in bpy.data.objects:
        if obj.type != "MESH":
            continue
        # Get world-space bounding box corners
        for corner in obj.bound_box:
            world = obj.matrix_world @ mathutils.Vector(corner)
            x_min = min(x_min, world.x)
            x_max = max(x_max, world.x)
            # For top-down view in Blender (Z-up), Y is the "depth" axis
            y_min = min(y_min, world.y)
            y_max = max(y_max, world.y)

    return x_min, x_max, y_min, y_max


def setup_workbench_render(render_config):
    """Configure Workbench engine for fast textured rendering."""
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"

    # Solid mode settings
    scene.display.shading.light = "FLAT"
    scene.display.shading.color_type = "TEXTURE"
    scene.display.shading.show_shadows = False
    scene.display.shading.show_cavity = False

    # Transparent background
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"

    # Resolution
    ppm = render_config.get("pixels_per_meter", 32)
    x_min, x_max, y_min, y_max = get_scene_bounds()
    padding = render_config.get("camera_padding", 5.0)

    width_m = (x_max - x_min) + 2 * padding
    height_m = (y_max - y_min) + 2 * padding

    scene.render.resolution_x = int(width_m * ppm)
    scene.render.resolution_y = int(height_m * ppm)
    scene.render.resolution_percentage = 100

    return x_min, x_max, y_min, y_max, padding


def setup_camera(x_min, x_max, y_min, y_max, padding):
    """Create orthographic top-down camera."""
    # Remove existing cameras
    for obj in list(bpy.data.objects):
        if obj.type == "CAMERA":
            bpy.data.objects.remove(obj, do_unlink=True)

    cam_data = bpy.data.cameras.new("PipelineCamera")
    cam_data.type = "ORTHO"

    width_m = (x_max - x_min) + 2 * padding
    height_m = (y_max - y_min) + 2 * padding
    cam_data.ortho_scale = max(width_m, height_m)

    cam_obj = bpy.data.objects.new("PipelineCamera", cam_data)
    bpy.context.scene.collection.objects.link(cam_obj)
    bpy.context.scene.camera = cam_obj

    # Position camera above center, looking down (Blender Z-up)
    center_x = (x_min + x_max) / 2
    center_y = (y_min + y_max) / 2
    cam_obj.location = (center_x, center_y, 100)  # High above
    cam_obj.rotation_euler = (0, 0, 0)  # Looking down -Z

    return cam_obj


def hide_objects_outside_floor(floor_config):
    """Hide mesh objects whose Z-center (Blender Z-up) is outside the floor range.

    Note: In Blender Z is up, but our config uses Y (glTF convention).
    Blender Z maps to glTF Y. So floor y_min/y_max correspond to Blender Z.
    """
    y_min = floor_config.get("y_min", float("-inf"))
    y_max = floor_config.get("y_max", float("inf"))

    hidden = 0
    shown = 0
    for obj in bpy.data.objects:
        if obj.type != "MESH":
            continue
        # Object center Z in world space (Blender Z = height)
        z_center = obj.matrix_world.translation.z
        if z_center < y_min or z_center > y_max:
            obj.hide_render = True
            hidden += 1
        else:
            obj.hide_render = False
            shown += 1

    print(f"  Floor [{y_min:.1f}, {y_max:.1f}]: showing {shown}, hiding {hidden} objects")


def render_floor(floor_config, output_dir, render_bounds):
    """Render a single floor to PNG."""
    floor_id = floor_config["id"]
    output_path = os.path.join(output_dir, f"{floor_id}.png")

    hide_objects_outside_floor(floor_config)

    bpy.context.scene.render.filepath = output_path
    bpy.ops.render.render(write_still=True)

    print(f"  Rendered floor '{floor_id}' -> {output_path}")


# ─────────────────────────────────────────────────────
# glTF export
# ─────────────────────────────────────────────────────

def export_glb(output_dir, map_id):
    """Export the scene as GLB."""
    # Show all objects for export
    for obj in bpy.data.objects:
        obj.hide_render = False

    output_path = os.path.join(output_dir, f"{map_id}.glb")

    bpy.ops.export_scene.gltf(
        filepath=output_path,
        export_format="GLB",
        use_selection=False,
        export_apply=True,
        export_materials="PLACEHOLDER",
        export_draco_mesh_compression_enable=True,
    )

    print(f"  Exported GLB -> {output_path}")


# ─────────────────────────────────────────────────────
# Entity export
# ─────────────────────────────────────────────────────

def export_entities(output_dir):
    """Export entity positions (health packs, spawns) as JSON.

    Entities imported by io_scene_owm are typically Empty objects.
    This is a best-effort extraction — GUIDs need manual mapping.
    """
    entities = {
        "health_packs": [],
        "spawns": [],
        "objectives": [],
        "empties": [],  # Raw empty positions for manual review
    }

    for obj in bpy.data.objects:
        if obj.type == "EMPTY":
            loc = obj.matrix_world.translation
            entities["empties"].append({
                "name": obj.name,
                "x": round(loc.x, 3),
                "y": round(loc.z, 3),  # Blender Z -> glTF Y (height)
                "z": round(loc.y, 3),  # Blender Y -> glTF Z (depth)
            })

    output_path = os.path.join(output_dir, "entities.json")
    with open(output_path, "w") as f:
        json.dump(entities, f, indent=2)

    print(f"  Exported {len(entities['empties'])} entities -> {output_path}")


# ─────────────────────────────────────────────────────
# Main
# ─────────────────────────────────────────────────────

def main():
    args = parse_args()

    print(f"\n{'='*60}")
    print(f"  Scuffed Map Pipeline — Blender Export")
    print(f"{'='*60}\n")

    # Parse config
    config = parse_toml_simple(args.config)
    map_info = config.get("map", {})
    cleanup_config = config.get("cleanup", {})
    render_config = config.get("render", {})
    floors = config.get("floors", [])

    map_id = map_info.get("id", "unknown")
    print(f"Map: {map_info.get('name', 'Unknown')} ({map_id})")

    # Create output directory
    os.makedirs(args.output, exist_ok=True)

    # Cleanup
    print("\nStep 1: Scene cleanup")
    cleanup_scene(cleanup_config)

    # Export GLB if requested
    if args.export_glb:
        print("\nStep 2: GLB export")
        export_glb(args.output, map_id)

    # Render floors
    if not args.skip_render:
        if not floors:
            print("\nERROR: No floors defined in config. Run 'detect-floors' first.")
            sys.exit(1)

        print(f"\nStep 3: Rendering {len(floors)} floors (Workbench engine)")
        bounds = setup_workbench_render(render_config)
        setup_camera(*bounds)

        for floor in floors:
            render_floor(floor, args.output, bounds)

    # Export entities
    print("\nStep 4: Entity export")
    export_entities(args.output)

    # Restore visibility
    for obj in bpy.data.objects:
        obj.hide_render = False

    print(f"\n{'='*60}")
    print(f"  Done! Output in: {args.output}")
    print(f"{'='*60}\n")


if __name__ == "__main__":
    main()
