"""Shared Blender helpers: import ue-export output (PSK + PSA + material json) into the current scene."""
import os, json
import bpy
import numpy as np

MAX_TEX = int(os.environ.get("MAX_TEX", 2048))


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.context.scene.render.fps = 30
    import addon_utils
    addon_utils.enable("bl_ext.blender_org.io_scene_psk_psa")


def import_item(mesh_json, psas=(), name=None):
    """Import <mesh_json>'s PSK (metres), build PBR materials, import PSA sequences as actions. Returns (armature, meshes)."""
    mesh_json = os.path.abspath(mesh_json)
    d = os.path.dirname(mesh_json)
    meta = json.load(open(mesh_json))
    before = set(bpy.data.objects)
    base = os.path.join(d, os.path.basename(meta["asset"]))
    bpy.ops.psk.import_file(filepath=base + (".pskx" if os.path.exists(base + ".pskx") else ".psk"), scale=0.01,
                            should_import_materials=True, bone_length=0.02)
    new = [o for o in bpy.data.objects if o not in before]
    arm = next(o for o in new if o.type == "ARMATURE")
    meshes = [o for o in new if o.type == "MESH"]
    arm.name = name or os.path.splitext(os.path.basename(mesh_json))[0]
    bake_scale(arm, meshes)
    for p in psas:
        bpy.context.view_layer.objects.active = arm
        r = bpy.ops.psa.import_all(filepath=os.path.abspath(p), translation_scale=0.01, fps_source="SCENE",
                                   should_write_scale_keys=False, should_overwrite=True)
        if r != {"FINISHED"}:
            print("PSA FAIL", p, r)
    arm.animation_data_create()
    for act in bpy.data.actions:
        act.use_fake_user = True
    arm.animation_data.action = None
    build_materials(meta, d, meshes)
    return arm, meshes


def bake_scale(arm, meshes):
    """PSK import stores cm->m as object scale 0.01. Bake it into bone + vertex data so exported nodes have scale 1."""
    for o in bpy.data.objects:
        o.select_set(False)
    mats = {m: m.matrix_world.copy() for m in meshes}
    for m in meshes:
        m.parent = None
        m.matrix_world = mats[m]
    bpy.context.view_layer.objects.active = arm
    arm.select_set(True)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    arm.select_set(False)
    for m in meshes:
        bpy.context.view_layer.objects.active = m
        m.select_set(True)
        bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
        m.select_set(False)
        m.parent = arm
        m.matrix_parent_inverse = arm.matrix_world.inverted()


def load_img(d, fname, colorspace, flip_green=False):
    img = bpy.data.images.load(os.path.join(d, fname))
    img.colorspace_settings.name = colorspace
    if max(img.size) > MAX_TEX:
        img.scale(*[min(v, MAX_TEX) for v in img.size])
    if flip_green:  # UE normals are DirectX (Y-), glTF is OpenGL (Y+)
        px = np.empty(len(img.pixels), dtype=np.float32)
        img.pixels.foreach_get(px)
        px[1::4] = 1.0 - px[1::4]
        img.pixels.foreach_set(px)
        img.pack()
    return img


def pick(tex, keys):
    for k in keys:
        if k in tex:
            return tex[k]


def build_materials(meta, d, meshes):
    for mdef in meta["materials"]:
        old = [m for m in bpy.data.materials if m.name.split(".")[0] in (mdef["slot"], mdef.get("material"))]
        for o in old:
            o.name = "_old_" + o.name
        mat = bpy.data.materials.new(mdef.get("material") or mdef["slot"])
        mat.use_nodes = True
        nt = mat.node_tree
        nt.nodes.clear()
        out = nt.nodes.new("ShaderNodeOutputMaterial")
        bsdf = nt.nodes.new("ShaderNodeBsdfPrincipled")
        nt.links.new(bsdf.outputs[0], out.inputs[0])
        tex = mdef.get("textures", {})
        dif = pick(tex, ["PM_Diffuse", "Texture"])
        nrm = pick(tex, ["PM_Normals", "Normals"])
        orm = pick(tex, ["PM_SpecularMasks", "ORM"])
        col = mdef.get("colors", {}).get("Albedo Tint")
        if dif:
            t = nt.nodes.new("ShaderNodeTexImage"); t.image = load_img(d, dif, "sRGB")
            src = t.outputs["Color"]
            if col:
                mix = nt.nodes.new("ShaderNodeMix"); mix.data_type = "RGBA"; mix.blend_type = "MULTIPLY"
                mix.inputs[0].default_value = 1.0
                mix.inputs[7].default_value = (col[0], col[1], col[2], 1.0)
                nt.links.new(src, mix.inputs[6]); src = mix.outputs[2]
            nt.links.new(src, bsdf.inputs["Base Color"])
        if nrm:
            t = nt.nodes.new("ShaderNodeTexImage"); t.image = load_img(d, nrm, "Non-Color", flip_green=True)
            n = nt.nodes.new("ShaderNodeNormalMap")
            nt.links.new(t.outputs["Color"], n.inputs["Color"]); nt.links.new(n.outputs[0], bsdf.inputs["Normal"])
        if orm:  # UE ORM: R=AO, G=roughness, B=metallic == glTF occlusion / metallicRoughness layout
            t = nt.nodes.new("ShaderNodeTexImage"); t.image = load_img(d, orm, "Non-Color")
            sep = nt.nodes.new("ShaderNodeSeparateColor")
            nt.links.new(t.outputs["Color"], sep.inputs[0])
            nt.links.new(sep.outputs[1], bsdf.inputs["Roughness"])
            nt.links.new(sep.outputs[2], bsdf.inputs["Metallic"])
            g = bpy.data.node_groups.get("glTF Material Output") or bpy.data.node_groups.new("glTF Material Output", "ShaderNodeTree")
            if "Occlusion" not in [i.name for i in g.interface.items_tree]:
                g.interface.new_socket("Occlusion", in_out="INPUT", socket_type="NodeSocketFloat")
            gn = nt.nodes.new("ShaderNodeGroup"); gn.node_tree = g
            nt.links.new(sep.outputs[0], gn.inputs["Occlusion"])
        for m in meshes:
            for s in m.material_slots:
                if s.material in old:
                    s.material = mat
