"""Import a MultiCAD GLB into an empty Blender scene and report what Blender
sees: every mesh object with its parent chain, world bounds in meters, shared
mesh data and material base color. Developer verification only; run with

    blender -b --factory-startup --python-exit-code 1 \
        --python tools/blender/glb_import_report.py -- INPUT.glb REPORT.json
"""
import json
import sys

import bpy
from mathutils import Vector

source, report = sys.argv[sys.argv.index("--") + 1:][:2]
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=source)
scene = bpy.context.scene
objects = []
for obj in sorted(scene.objects, key=lambda o: o.name):
    entry = {"name": obj.name, "type": obj.type}
    chain, parent = [], obj.parent
    while parent is not None:
        chain.append(parent.name)
        parent = parent.parent
    entry["parents"] = chain
    if obj.type == "MESH":
        corners = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
        entry["min_m"] = [min(c[i] for c in corners) for i in range(3)]
        entry["max_m"] = [max(c[i] for c in corners) for i in range(3)]
        entry["mesh"] = obj.data.name
        entry["mesh_users"] = obj.data.users
        entry["triangles"] = sum(len(p.vertices) - 2 for p in obj.data.polygons)
        material = obj.active_material
        if material is not None and material.use_nodes:
            bsdf = material.node_tree.nodes.get("Principled BSDF")
            if bsdf is not None:
                entry["base_color"] = list(bsdf.inputs["Base Color"].default_value)
    objects.append(entry)
with open(report, "w", encoding="utf-8") as out:
    json.dump(
        {
            "blender": bpy.app.version_string,
            "unit_system": scene.unit_settings.system,
            "scale_length": scene.unit_settings.scale_length,
            "objects": objects,
        },
        out,
        indent=2,
    )
