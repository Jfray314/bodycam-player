"""Render the arms holding a gun at frame 0 of an FP clip, straight from ue-export output (verifies attach bone + orientation).
blender --background --python preview.py -- <out_dir> <Gun> <clip> <out.png> [attach_bone]
"""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bpy, mathutils
import ueblender

out, gun, clip, png, *rest = sys.argv[sys.argv.index("--") + 1:]
bone = rest[0] if rest else "ik_hand_gun"
ueblender.reset()
arms, _ = ueblender.import_item(f"{out}/arms/arms.json", [f"{out}/fp/{gun}/{clip}.psa"], "arms")
g, _ = ueblender.import_item(f"{out}/guns/{gun}/{gun}.json", [], "gun")
ueblender.attach_parts(g, os.path.abspath(f"{out}/guns/{gun}"))
arms.animation_data.action = None if os.environ.get("REST") else bpy.data.actions[clip]
bpy.context.scene.frame_set(0)
bpy.context.view_layer.update()
pb = arms.pose.bones[bone]
g.matrix_world = arms.matrix_world @ pb.matrix          # identity attach to the bone frame
print("attach", bone, pb.matrix.to_translation(), "gun", g.matrix_world.to_euler())

for b in ("head","hand_r","hand_l","ik_hand_gun","upperarm_r"):
    print("BONE", b, tuple(round(v,3) for v in (arms.matrix_world @ arms.pose.bones[b].matrix).to_translation()))
# camera at the FP_Camera socket (head bone), looking along the arms' forward
head = arms.matrix_world @ arms.pose.bones["head"].matrix
cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam")); bpy.context.collection.objects.link(cam)
cam.data.lens = float(os.environ.get("LENS", 24))
eye = head.to_translation()
fwd = mathutils.Vector(eval(os.environ.get("FWD", "(0,-1,0)")))
cam.location = mathutils.Vector(eval(os.environ["CAM"])) if "CAM" in os.environ else eye + mathutils.Vector((0, 0, 0.02))
tgt = mathutils.Vector(eval(os.environ["TGT"])) if "TGT" in os.environ else cam.location + fwd
cam.rotation_euler = (tgt - cam.location).to_track_quat("-Z", "Y").to_euler()
bpy.context.scene.camera = cam
sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN")); sun.data.energy = 6; sun.rotation_euler = (0.8, 0.2, 0.5)
bpy.context.collection.objects.link(sun)
w = bpy.data.worlds.new("w"); w.use_nodes = True; w.node_tree.nodes["Background"].inputs[1].default_value = 1.5
w.node_tree.nodes["Background"].inputs[0].default_value = (0.5, 0.55, 0.6, 1)
bpy.context.scene.world = w
sc = bpy.context.scene
sc.render.engine = "CYCLES"; sc.cycles.samples = 24; sc.cycles.device = "CPU"
sc.render.resolution_x, sc.render.resolution_y = 960, 540
sc.render.filepath = os.path.abspath(png)
bpy.ops.render.render(write_still=True)
print("RENDERED", png)
