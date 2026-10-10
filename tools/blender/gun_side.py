"""Side/top ortho render of a gun glb (what the game loads), to audit missing parts.
blender -b -P gun_side.py -- <gun.glb> <out.png> [side|top]"""
import sys, os, bpy, mathutils
glb, png, *r = sys.argv[sys.argv.index("--") + 1:]
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
meshes = [o for o in bpy.data.objects if o.type == "MESH" and not o.name.startswith("Icosphere")]
pts = [o.matrix_world @ mathutils.Vector(c) for o in meshes for c in o.bound_box]
lo = mathutils.Vector([min(p[i] for p in pts) for i in range(3)])
hi = mathutils.Vector([max(p[i] for p in pts) for i in range(3)])
print("BOUNDS", [round(v, 3) for v in lo], [round(v, 3) for v in hi], [o.name for o in meshes])
c = (lo + hi) / 2
cam = bpy.data.objects.new("c", bpy.data.cameras.new("c")); bpy.context.collection.objects.link(cam)
cam.data.type = "ORTHO"; cam.data.ortho_scale = max(hi - lo) * 1.15
ext = hi - lo
ax = min(range(3), key=lambda i: ext[i])  # side view = look along the gun's narrowest axis
if (r[0] if r else "side") == "top":
    ax = sorted(range(3), key=lambda i: ext[i])[1]
off = mathutils.Vector([10 if i == ax else 0 for i in range(3)])
cam.location = c + off
cam.rotation_euler = off.to_track_quat("Z", "Y" if ax != 1 else "X").to_euler()
bpy.context.scene.camera = cam
w = bpy.data.worlds.new("w"); w.use_nodes = True
w.node_tree.nodes["Background"].inputs[0].default_value = (.5, .55, .6, 1); bpy.context.scene.world = w
s = bpy.context.scene; s.render.engine = "BLENDER_WORKBENCH"
s.display.shading.light = "STUDIO"; s.display.shading.color_type = "TEXTURE"
s.render.resolution_x, s.render.resolution_y = 900, 900; s.render.filepath = os.path.abspath(png)
bpy.ops.render.render(write_still=True)
