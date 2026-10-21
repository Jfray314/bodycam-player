"""Blender headless: render attachment glbs that have no DT_WeaponSkins ItemIcons into icon pngs (game icon look: lit part on
black, 3/4 side view). blender --background --python icon_render.py -- <out_dir> <glb>..."""
import bpy, sys, os, math
from mathutils import Vector

args = sys.argv[sys.argv.index("--") + 1:]
out, glbs = args[0], args[1:]
os.makedirs(out, exist_ok=True)
for path in glbs:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=path)
    # attach-frame glbs are up = +Z in glTF space, which the importer maps to Blender -Y: stand the part up
    for o in bpy.context.scene.objects:
        if o.parent is None:
            o.rotation_mode = "XYZ"; o.rotation_euler.x += math.pi / 2
    bpy.context.view_layer.update()
    objs = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    if not objs:
        continue
    pts = [o.matrix_world @ Vector(c) for o in objs for c in o.bound_box]
    lo = Vector((min(p.x for p in pts), min(p.y for p in pts), min(p.z for p in pts)))
    hi = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
    centre, size = (lo + hi) / 2, max((hi - lo).length, 0.01)
    scn = bpy.context.scene
    scn.render.engine = "BLENDER_EEVEE_NEXT" if "BLENDER_EEVEE_NEXT" in {e.identifier for e in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items} else "BLENDER_EEVEE"
    scn.render.resolution_x = scn.render.resolution_y = 512
    scn.render.film_transparent = False
    world = bpy.data.worlds.new("w"); world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[0].default_value = (0, 0, 0, 1); scn.world = world
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    scn.collection.objects.link(cam); scn.camera = cam
    cam.data.lens = 50
    d = Vector((1.0, -1.4, 0.6)).normalized() * size * 1.9
    cam.location = centre + d
    cam.rotation_euler = (centre - cam.location).to_track_quat("-Z", "Y").to_euler()
    for loc, e in (((1, -1, 1.5), 300), ((-1.5, 0.5, 0.5), 120), ((0, 1.5, -0.5), 80)):
        l = bpy.data.objects.new("l", bpy.data.lights.new("l", "POINT")); l.data.energy = e * size * size * 4
        l.location = centre + Vector(loc) * size * 1.5; scn.collection.objects.link(l)
    scn.render.filepath = os.path.join(out, "gen_" + os.path.splitext(os.path.basename(path))[0] + ".png")
    bpy.ops.render.render(write_still=True)
