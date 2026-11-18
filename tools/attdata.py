"""Attachment data -> assets/attachments.json (read by game/src/attach.rs).
Inputs: <models>/_work_att/attall_meta.json (tools/attall.py), <models>/_work_att/att/<glb>/<glb>.json (mesh sockets),
<models>/_work/guns/<gun>/<gun>.json (receiver sockets), GE_AttachmentStat_* (stat modifiers), optic MIs (reticle / magnification).
Transforms: UE (x,y,z) cm -> (x,-y,z) m, rotations as quaternions xyzw (as ueblender.attach_parts). Receiver sockets carry their bone;
item sockets are in the item mesh frame. The game resolves the mount tree (receiver -> parts -> parts) per loadout.
python -I tools/attdata.py [<models_dir>] [<extracted_json_root>]"""
import json, os, sys, glob
import numpy as np
M = (sys.argv[1] if len(sys.argv) > 1 else os.environ.get("BODYCAM_ASSETS", os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "assets-local")) + "/models").rstrip("/")
X = (sys.argv[2] if len(sys.argv) > 2 else os.environ.get("BODYCAM_EXTRACTED", os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "extracted")) + "/json") + "/Content/"
HERE = os.path.dirname(os.path.abspath(__file__))
F = np.diag([1, -1, 1.0])

def rot(pyr):  # UE (pitch, yaw, roll) deg, as ueblender: Rz(yaw) Ry(pitch) Rx(-roll), mirrored by F
    p, y, r = (np.radians(v) for v in pyr)
    Rz = np.array([[np.cos(y), -np.sin(y), 0], [np.sin(y), np.cos(y), 0], [0, 0, 1]])
    Ry = np.array([[np.cos(p), 0, np.sin(p)], [0, 1, 0], [-np.sin(p), 0, np.cos(p)]])
    Rx = np.array([[1, 0, 0], [0, np.cos(-r), -np.sin(-r)], [0, np.sin(-r), np.cos(-r)]])
    return F @ (Rz @ Ry @ Rx) @ F

def quat(R):
    from math import sqrt
    t = np.trace(R)
    if t > 0:
        s = sqrt(t + 1) * 2; q = [(R[2, 1] - R[1, 2]) / s, (R[0, 2] - R[2, 0]) / s, (R[1, 0] - R[0, 1]) / s, s / 4]
    else:
        i = int(np.argmax(np.diag(R))); j, k = (i + 1) % 3, (i + 2) % 3
        s = sqrt(R[i, i] - R[j, j] - R[k, k] + 1) * 2; q = [0, 0, 0, 0]
        q[i] = s / 4; q[j] = (R[j, i] + R[i, j]) / s; q[k] = (R[k, i] + R[i, k]) / s; q[3] = (R[k, j] - R[j, k]) / s
    return [round(float(v), 5) for v in q]

def xf(s, bone=None):
    d = {"loc": [round(s["loc"][0] / 100, 5), round(-s["loc"][1] / 100, 5), round(s["loc"][2] / 100, 5)], "rot": quat(rot(s["rot"]))}
    if bone: d["bone"] = bone
    return d

slug = lambda s: s.lower().replace(" ", "_")
meta = json.load(open(f"{M}/_work_att/attall_meta.json"))
wi = json.load(open(X + "BodycamCore/ItemsDefinition/Gameplay/DT_WeaponItems.json"))["exports"][0]["properties"]["_rows"]

# GE_AttachmentStat_* -> [(attribute, op, value)]; op: Multiplicitive -> "mul", Additive -> "add", Override -> "set".
ge = {os.path.basename(f)[:-5]: f for f in glob.glob(X + "BodycamCore/AbilitySystem/GameplayEffects/Weapons/Stats/Attachments/*.json")}
OPS = {"EGameplayModOp::Multiplicitive": "mul", "EGameplayModOp::Additive": "add", "EGameplayModOp::Override": "set"}
def effects(names):
    out = []
    for n in names:
        if n not in ge: continue
        for e in json.load(open(ge[n]))["exports"]:
            for m in e.get("properties", {}).get("Modifiers", []):
                a = m["Attribute"]["AttributeName"]
                v = m.get("ModifierMagnitude", {}).get("ScalableFloatMagnitude", {}).get("Value")
                if a and v is not None and m["ModifierOp"] in OPS: out.append([a, OPS[m["ModifierOp"]], round(v, 5)])
    return out

# Reticles: optic MI chain (OpticReticleMaterial: Magnification, ScreenHeightFraction, defaults) + the gun's default Reticle item
# per optic type (DT_WeaponItems DefaultAttachments: "Dot" / "Canted Dot" / "Magnified Dot MK1 Red").
def props(path):
    p = X + path.split(".")[0].replace("/Game/", "") + ".json"
    return json.load(open(p))["exports"][0]["properties"] if os.path.exists(p) else None
def chain(path):
    out = {}
    while path and (pr := props(path)):
        for k in ("ScalarParameterValues", "VectorParameterValues", "TextureParameterValues"):
            for v in pr.get(k, []): out.setdefault(v["ParameterInfo"]["Name"], v["ParameterValue"])
        path = pr.get("Parent")
    return out
RET = {"regular": "BodycamScope/Reticles/Regular/MI_ReticleData_Dot_Red", "canted": "BodycamScope/Reticles/Regular/MI_ReticleData_Dot_Red",
       "magnified": "BodycamScope/Reticles/Magnified/MI_ReticleMagnifiedData_DotMK1_Red"}
lib = {e["id"]: e for e in json.load(open(f"{M}/_work_att/attall.json"))}
def reticle(glb, cls, socks):
    mj = X + lib[glb]["mesh"][len("Bodycam/Content/"):] + ".json"
    if not os.path.exists(mj): return None
    s = json.dumps(json.load(open(mj)))
    # no user data (older meshes, e.g. ACRO P239): BP_StaticReticleAttachment keeps the reticle mesh's own M_ReticleData -> defaults
    if "OpticReticleMaterial" in s: optic = chain(s.split('"OpticReticleMaterial": "')[1].split('"')[0])
    elif "reticle" in socks or "reticle_canted" in socks: optic = {}
    else: return None
    kind = "magnified" if "Magnification" in optic else "canted" if cls.startswith("Canted") else "regular"
    d = chain(RET[kind])
    tex = d.get("ReticleTexture") or optic.get("ReticleTexture")
    if not tex: return None
    c = d.get("ReticleColor") or d.get("ReticleIlluminationColor") or optic.get("ReticleColor") or {"r": 1, "g": 0, "b": 0}
    so = d.get("ReticleScaleAndOffset") or optic.get("ReticleScaleAndOffset") or {"r": 1, "g": 1, "b": 0, "a": 0}
    r = {"tex": tex.split("/")[-1].split(".")[0], "tex_path": tex, "color": [c["r"], c["g"], c["b"]],
         "intensity": d.get("ReticleIlluminationIntensity", d.get("ReticleIlluminationContrast", optic.get("ReticleIlluminationIntensity", 1.0))),
         "scale": [so["r"], so["g"], so["b"], so["a"]]}
    if "Magnification" in optic:
        # BP_StaticMagnifiedOpticAttachment: SetMagnification(Magnification, ScreenHeightFraction); M_ScopeOpticReticle front-lens vignette
        r["mag"], r["screen_fraction"] = optic["Magnification"], optic.get("ScreenHeightFraction", 0.5)
        r["vig"] = [optic.get("FrontLensVignetteIntensity", 0.0), optic.get("FrontLensVignetteMaskRadius", 0.4), optic.get("FrontLensVignetteMaskDensity", 2.0)]
    return r

items, missing = {}, []
for i, it in meta["items"].items():
    j = f"{M}/_work_att/att/{it['glb']}/{it['glb']}.json"
    if not os.path.exists(f"{M}/attachments/{it['glb']}.glb") or not os.path.exists(j):
        missing.append(i); continue
    socks = {s["name"].lower(): xf(s) for s in json.load(open(j))["sockets"]}
    e = {"cls": it["cls"], "slot": slug(it["cls"].split(".")[0]), "label": it["label"], "glb": it["glb"], "sockets": socks, "fx": effects(it["effects"])}
    # WEP.GetActiveGripData: Cosmetic.Weapon.Anim.Idle (left-hand pose clip) of a part; the hand goes to a part's GripPose socket
    grip = (wi.get(it["gp"], {}).get("GameplayDataDefinition", {}).get("CosmeticEffects") or {}).get("Cosmetic.Weapon.Anim.Idle")
    if grip: e["grip"] = grip.rsplit(".", 1)[-1]
    if it["icon"]: e["icon"] = it["icon"]
    elif os.path.exists(f"{M}/../ui/tex/icons/gen_{it['glb']}.png"): e["icon"] = "gen_" + it["glb"]  # no ItemIcons: tools/blender/icon_render.py
    if it["cls"].split(".")[0] in ("Optic", "Canted Optic") and (r := reticle(it["glb"], it["cls"], socks)): e["reticle"] = r
    items[i] = e

guns = {}
for g in sorted(os.listdir(f"{M}/_work/guns")):
    gj = f"{M}/_work/guns/{g}/{g}.json"
    if os.path.exists(gj):
        guns[g] = {s["name"].lower(): xf(s, s["bone"]) for s in json.load(open(gj))["sockets"]}
FOLDER = {"m4a1": "M4A1", "mglock17": "Glock17", "glock": "Glock17", "akm": "Akm", "deagle": "Deagle", "fal": "FAL", "mossberg590": "Mossberg590", "remington700": "Remington700",
          "revolver": "Revolver", "scar-h": "SCAR-H", "svd": "SVD", "uzi": "UZI", "mp9": "MP9", "vector": "Vector", "sg5-k": "MP5", "benelli": "Benelli", "crossbow": "Crossbow", "vaiga": "Vaiga"}
rows = {}  # weapon row -> gun glb (mirrors viewmodel::gun_model)
for f in glob.glob(os.path.join(HERE, "..", "assets", "weapons", "*.json")):
    w = json.load(open(f)); r = w["row"]
    if r in ("CZ-P-09", "Kobra", "M1911"): rows[r] = r; continue
    fire = (w["anim_set"].get("Fire") or [{}])[0]
    path = (fire.get("Weapon") or fire.get("Character") or {}).get("path", "")
    if "/Animations/" in path and (g := FOLDER.get(path.split("/Animations/")[1].split("/")[0].lower())): rows[r] = g
# Baked default parts (gun glb nodes named after the DT_NewShopItem row, spaces -> '_'; tools/attachments.py + ue-export).
baked = {g: [p["row"].replace(" ", "_") for p in a["parts"]] for g, a in json.load(open(f"{M}/_work/attachments.json")).items()}
compat = {r: [i for i in v if i in items] for r, v in meta["compat"].items()}
defaults = {r: [i for i in v if i in items] for r, v in meta["defaults"].items()}
# ids of the old curated library (settings saved before the full set) -> skin ids
old = f"{M}/_work_att/attlib.json"
aliases = {e["id"]: e["row"].lower().replace(" ", "_") for e in json.load(open(old))} if os.path.exists(old) else {}
aliases = {k: __import__("re").sub(r"[^a-z0-9]+", "_", v).strip("_") for k, v in aliases.items()}
# weapon row -> its class under Item.PrimaryWeapon / SecondaryWeapon ("Assault Rifles", "Pistols"...): the SWAP screen tabs
classes = {r: wi[r]["ItemClassID"].split(".")[2] for r in meta["compat"] if r in wi and wi[r]["ItemClassID"].count(".") >= 2}
json.dump({"items": items, "guns": guns, "rows": rows, "compat": compat, "defaults": defaults, "baked": baked, "aliases": aliases, "classes": classes},
          open(os.path.join(HERE, "..", "assets", "attachments.json"), "w"), separators=(",", ":"))
print(len(items), "items,", len(missing), "without glb;", {r: len(v) for r, v in compat.items()})
print("reticle textures:", sorted({e["reticle"]["tex_path"] for e in items.values() if "reticle" in e}))

# Reticle textures: export each listed path once with ue-export `tex Bodycam/Content/BodycamScope/Textures/...` into
# assets-local/models/attachments/reticles; this turns the black-background masks into white + alpha = luminance (512 px).
from PIL import Image  # noqa: E402
out = f"{M}/attachments/reticles"
for tex in {e["reticle"]["tex"] for e in items.values() if "reticle" in e}:
    f = os.path.join(out, tex + ".png")
    if not os.path.exists(f):
        print("reticle texture missing:", tex)
    elif (px := Image.open(f)).mode != "RGBA" or min(px.getchannel("A").getextrema()) > 0 and px.convert("RGB").getextrema() != ((255, 255),) * 3:
        # raw export (RGB, or RGBA with an opaque alpha like T_Reticle_DotMK1): black-background mask -> white + alpha = luminance
        im = px.convert("RGB").resize((512, 512), Image.LANCZOS)
        a = im.convert("L").point(lambda v: min(255, v * 2))
        Image.merge("RGBA", (Image.new("L", im.size, 255),) * 3 + (a,)).save(f)
