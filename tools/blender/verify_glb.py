"""Open each .glb in headless Blender and print bone count, clips (name, duration) and material texture assignments.
blender --background --python verify_glb.py -- a.glb [b.glb ...]
"""
import sys
import bpy

for path in sys.argv[sys.argv.index("--") + 1:]:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=path)
    arms = [o for o in bpy.data.objects if o.type == "ARMATURE"]
    fps = bpy.context.scene.render.fps
    print("VERIFY", path)
    print("  armatures", [(a.name, len(a.data.bones)) for a in arms], "meshes", [m.name for m in bpy.data.objects if m.type == "MESH"])
    clips = sorted((a.name, round((a.frame_range[1] - a.frame_range[0]) / fps, 3)) for a in bpy.data.actions)
    print("  clips", len(clips), clips[:6], "..." if len(clips) > 6 else "")
    for m in bpy.data.materials:
        imgs = sorted({n.image.name for n in m.node_tree.nodes if n.type == "TEX_IMAGE" and n.image}) if m.node_tree else []
        print("  material", m.name, imgs)
