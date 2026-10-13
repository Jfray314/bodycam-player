"""Shared Blender helpers: import ue-export output (PSK + PSA + material json) into the current scene."""
import os, json, math
import bpy
import numpy as np
import psa_scale

BUILT = {}
FACTORS = {}  # material name -> {roughnessFactor, metallicFactor}; applied to the glb JSON after export (Blender drops factors when a texture is linked)
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
    psk = base + (".pskx" if os.path.exists(base + ".pskx") else ".psk")
    for p in psas:
        bpy.context.view_layer.objects.active = arm
        ratio = psa_scale.trans_ratio(psk, p)  # != 1 when the clip's translations aren't in PSK units (Remington700: 100x)
        if ratio > 1.5:
            print("PSA translation ratio", ratio, os.path.basename(p))
        r = bpy.ops.psa.import_all(filepath=os.path.abspath(p), translation_scale=0.01 / ratio if ratio > 1.5 else 0.01, fps_source="SCENE",
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
    for i, mdef in enumerate(meta["materials"]):
        old = [m for m in bpy.data.materials if m.name.split(".")[0] in (mdef["slot"], mdef.get("material")) and m not in BUILT.values() and not m.name.startswith("_old_")]
        if not old:  # PSK material name != slot/MI name (guns.json material override, e.g. Glock17: PSK says M_Glock17_Shadow): match by slot index
            old = [s.material for m in meshes for j, s in enumerate(m.material_slots) if j == i and s.material and s.material not in BUILT.values()]
        for o in old:
            o.name = "_old_" + o.name
        key = mdef.get("material") or mdef["slot"]
        if key in BUILT:  # same MI used by several parts: reuse the finished material
            for m in meshes:
                for sl in m.material_slots:
                    if sl.material in old:
                        sl.material = BUILT[key]
            continue
        mat = bpy.data.materials.new(key)
        BUILT[key] = mat
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
        if dif and "specular" in dif.lower():  # e.g. M16A4 Stanag: the only "Texture" param is a specular map, not albedo
            dif = None
            bsdf.inputs["Base Color"].default_value = (0.04, 0.04, 0.04, 1)
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
            n.inputs["Strength"].default_value = mdef.get("scalars", {}).get("Normal Strength", 1.0)
            nt.links.new(t.outputs["Color"], n.inputs["Color"]); nt.links.new(n.outputs[0], bsdf.inputs["Normal"])
        if orm:  # UE ORM: R=AO, G=roughness, B=metallic == glTF occlusion / metallicRoughness layout
            t = nt.nodes.new("ShaderNodeTexImage"); t.image = load_img(d, orm, "Non-Color")
            sep = nt.nodes.new("ShaderNodeSeparateColor")
            nt.links.new(t.outputs["Color"], sep.inputs[0])
            nt.links.new(sep.outputs[1], bsdf.inputs["Roughness"])
            nt.links.new(sep.outputs[2], bsdf.inputs["Metallic"])
            # MI params: out = clamp(tex * Mult + Add, 0, Max); glTF only has a multiplicative factor (<=1), so use mean-based ratio
            px = np.empty(len(t.image.pixels), dtype=np.float32); t.image.pixels.foreach_get(px)
            sc = mdef.get("scalars", {})
            for ch, key, sock in ((1, "Roughness", "Roughness"), (2, "Metallic", "Metallic")):
                mean = float(px[ch::4].mean())
                tgt = mean * sc.get("Mult " + key, 1.0) + sc.get("Add " + key, 0.0)
                tgt = min(max(tgt, 0.0), sc.get("Max " + key, 1.0))
                if mean > 1e-4 and abs(tgt - mean) > 1e-3:
                    FACTORS.setdefault(mat.name, {})[key.lower() + "Factor"] = round(min(tgt / mean, 1.0), 4)
            g = bpy.data.node_groups.get("glTF Material Output") or bpy.data.node_groups.new("glTF Material Output", "ShaderNodeTree")
            if "Occlusion" not in [i.name for i in g.interface.items_tree]:
                g.interface.new_socket("Occlusion", in_out="INPUT", socket_type="NodeSocketFloat")
            gn = nt.nodes.new("ShaderNodeGroup"); gn.node_tree = g
            nt.links.new(sep.outputs[0], gn.inputs["Occlusion"])
        for m in meshes:
            for s in m.material_slots:
                if s.material in old:
                    s.material = mat


def attach_parts(arm, gun_dir):
    """Import <gun_dir>/parts/* (default attachments, see tools/attachments.py) and parent each to its socket's bone with the
    socket offset, so they follow bone animation (e.g. the magazine during reloads). Gun json supplies the sockets."""
    import mathutils
    work = os.path.dirname(os.path.dirname(gun_dir))
    att = json.load(open(os.path.join(work, "attachments.json"))).get(os.path.basename(gun_dir), {"parts": []})
    gun = json.load(open(os.path.join(gun_dir, os.path.basename(gun_dir) + ".json")))
    sockets = {s["name"].lower(): s for s in gun["sockets"]}
    bones = {b.name.lower(): b for b in arm.data.bones}
    global MAX_TEX
    MAX_TEX = min(MAX_TEX, 1024)  # attachment parts: smaller textures (file size)
    for part in att["parts"]:
        key = part["row"].replace(" ", "_")
        pj = os.path.join(gun_dir, "parts", key, key + ".json")
        if not os.path.exists(pj):
            print("PART MISSING", key)
            continue
        parm, meshes = import_item(pj, name="_part_" + key)
        # Missing socket -> last word of the class leaf (AK "Long_Barrel" -> "Barrel"; parts at the root origin were 21 cm short).
        s = sockets.get(part["socket"].lower()) or sockets.get(part["socket"].split("_")[-1].lower())
        bone = bones.get((s["bone"] if s else "root").lower()) or bones["root"]
        loc = mathutils.Vector((s["loc"][0], -s["loc"][1], s["loc"][2])) / 100 if s else mathutils.Vector()  # UE cm, Y mirrored like the PSK import
        S = mathutils.Matrix.Translation(loc)
        if s:  # json rot = UE (Pitch, Yaw, Roll) degrees; R = Rz(yaw) Ry(pitch) Rx(-roll), conjugated by the Y mirror (cancels the rotated Revolver Root bone)
            ry, rz, rx = (math.radians(v) for v in s["rot"])
            Rue = mathutils.Matrix.Rotation(rz, 3, "Z") @ mathutils.Matrix.Rotation(ry, 3, "Y") @ mathutils.Matrix.Rotation(-rx, 3, "X")
            F = mathutils.Matrix.Diagonal((1, -1, 1))
            S = S @ (F @ Rue @ F).to_4x4()
        for m in meshes:
            mw = m.matrix_world.copy()
            for mod in [x for x in m.modifiers if x.type == "ARMATURE"]:
                m.modifiers.remove(mod)
            m.vertex_groups.clear()
            m.parent = None
            m.matrix_world = mw
            m.parent = arm
            m.parent_type = "BONE"
            m.parent_bone = bone.name
            m.matrix_world = arm.matrix_world @ bone.matrix_local @ S @ mw
            m.name = key
        bpy.data.objects.remove(parm)
        print("ATTACHED", key, "->", bone.name, [round(v, 3) for v in loc])
