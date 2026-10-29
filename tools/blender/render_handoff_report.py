"""Import a MultiCAD render handoff GLB into an empty Blender scene and
report what Blender sees: mesh objects with parent chains, world bounds in
meters, shared mesh data and Principled BSDF values; the camera with its
projection, world location and view direction; lights with type, energy,
color and direction; and the scene's custom properties from glTF scene
extras. Developer verification only; run with

    blender -b --factory-startup --python-exit-code 1 \
        --python tools/blender/render_handoff_report.py -- INPUT.glb REPORT.json
"""
import json
import sys

import bpy
from mathutils import Vector

source, report = sys.argv[sys.argv.index("--") + 1:][:2]
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=source)
scene = bpy.context.scene


def parents(obj):
    chain, parent = [], obj.parent
    while parent is not None:
        chain.append(parent.name)
        parent = parent.parent
    return chain


def forward(obj):
    # Blender cameras and lights look along their local -Z axis.
    v = obj.matrix_world.to_3x3() @ Vector((0.0, 0.0, -1.0))
    v.normalize()
    return list(v)


objects, cameras, lights = [], [], []
for obj in sorted(scene.objects, key=lambda o: o.name):
    entry = {"name": obj.name, "type": obj.type, "parents": parents(obj)}
    if obj.type == "MESH":
        corners = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
        entry["min_m"] = [min(c[i] for c in corners) for i in range(3)]
        entry["max_m"] = [max(c[i] for c in corners) for i in range(3)]
        entry["mesh"] = obj.data.name
        entry["mesh_users"] = obj.data.users
        material = obj.active_material
        if material is not None and material.use_nodes:
            bsdf = material.node_tree.nodes.get("Principled BSDF")
            if bsdf is not None:
                entry["base_color"] = list(bsdf.inputs["Base Color"].default_value)
                entry["roughness"] = bsdf.inputs["Roughness"].default_value
                entry["metallic"] = bsdf.inputs["Metallic"].default_value
        objects.append(entry)
    elif obj.type == "CAMERA":
        cam = obj.data
        cameras.append(
            {
                "name": obj.name,
                "projection": cam.type,
                "angle_y": cam.angle_y,
                "ortho_scale": cam.ortho_scale,
                "location_m": list(obj.matrix_world.translation),
                "forward": forward(obj),
            }
        )
    elif obj.type == "LIGHT":
        light = obj.data
        lights.append(
            {
                "name": obj.name,
                "light_type": light.type,
                "energy": light.energy,
                "color": list(light.color),
                "location_m": list(obj.matrix_world.translation),
                "forward": forward(obj),
            }
        )
custom = {}
for key in scene.keys():
    value = scene[key]
    if hasattr(value, "to_list"):
        value = value.to_list()
    try:
        json.dumps(value)
    except TypeError:
        value = list(value) if hasattr(value, "__iter__") else str(value)
    custom[key] = value
with open(report, "w", encoding="utf-8") as out:
    json.dump(
        {
            "blender": bpy.app.version_string,
            "unit_system": scene.unit_settings.system,
            "scale_length": scene.unit_settings.scale_length,
            "objects": objects,
            "cameras": cameras,
            "lights": lights,
            "scene_properties": custom,
        },
        out,
        indent=2,
    )
