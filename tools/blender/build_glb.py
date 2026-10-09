"""Blender headless: ue-export output (PSK + PSA* + material json) -> .glb (skinned mesh, PBR textures, one glTF clip per PSA sequence).

blender --background --python build_glb.py -- <mesh.json> <out.glb> <psa> [<psa> ...]

env FRAME=viewer (default): metres, Y-up, item's UE +X (forward, = Blender -Y after PSK import) -> glTF -Z (Bevy forward).
env FRAME=attach (guns): raw UE-bone-local axes, metres, NO Y-up conversion and no root rotation, so the gun scene can be
    parented to the arms' `ik_hand_gun` joint with an identity transform.
env NO_MESH=1: animation-only skeleton (same joint names as arms.glb; load the clips next to the arms).
"""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bpy
import ueblender

mesh_json, out_glb, *psas = sys.argv[sys.argv.index("--") + 1:]
out_glb = os.path.abspath(out_glb)
ueblender.reset()
arm, meshes = ueblender.import_item(mesh_json, psas)

for o in [o for o in bpy.data.objects if o.type == "MESH" and o not in meshes]:  # bone-shape helper ("Icosphere") from the PSK importer
    bpy.data.objects.remove(o)
FRAME = os.environ.get("FRAME", "viewer")
if FRAME == "viewer":
    arm.rotation_euler = (0, 0, math.radians(180))  # PSK import: UE +X forward == Blender -Y; need +Y so glTF forward is -Z
if os.environ.get("NO_MESH"):
    for m in meshes:
        bpy.data.objects.remove(m)

bpy.ops.export_scene.gltf(filepath=out_glb, export_format="GLB", export_animation_mode="ACTIONS",
                          export_apply=False, export_yup=(FRAME == "viewer"), export_skins=True, export_texcoords=True,
                          export_normals=True, export_image_format="AUTO", export_force_sampling=True,
                          export_optimize_animation_size=True, export_anim_single_armature=True, export_cameras=False,
                          export_lights=False)
print("WROTE", out_glb, os.path.getsize(out_glb))
