"""Attachment mount data -> assets/attachments.json.
Inputs: <models>/_work/guns/<gun>/<gun>.json (receiver sockets), <models>/_work/attachments.json (default parts + their mount socket),
<models>/_work_att/parts/att/<gun>__<row>/*.json (static-mesh sockets of default parts: Muzzle/Underbarrel/Upperbarrel/Siderail...),
<models>/_work_att/att/<id>/<id>.json (library meshes; sockets Aimpoint/Reticle/Viewpoint/Light/Socket_Laser...).
All transforms are in gun-joint space (metres; UE (x,y,z) cm -> (x,-y,z)/100, as ueblender.attach_parts), rotations as quaternions xyzw.
python -I tools/attsockets.py [<models_dir>]"""
import json, os, sys, glob
import numpy as np
M = (sys.argv[1] if len(sys.argv) > 1 else "C:/code/bodycam-player/assets-local/models").rstrip("/")
HERE = os.path.dirname(os.path.abspath(__file__))
F = np.diag([1, -1, 1.0])

def rot(pyr):  # UE (pitch, yaw, roll) deg, as ueblender: Rz(yaw) Ry(pitch) Rx(-roll), mirrored by F
    p, y, r = (np.radians(v) for v in pyr)
    Rz = np.array([[np.cos(y), -np.sin(y), 0], [np.sin(y), np.cos(y), 0], [0, 0, 1]])
    Ry = np.array([[np.cos(p), 0, np.sin(p)], [0, 1, 0], [-np.sin(p), 0, np.cos(p)]])
    Rx = np.array([[1, 0, 0], [0, np.cos(-r), -np.sin(-r)], [0, np.sin(-r), np.cos(-r)]])
    return F @ (Rz @ Ry @ Rx) @ F

def mat(s):
    m = np.eye(4); m[:3, :3] = rot(s["rot"]); m[:3, 3] = np.array([s["loc"][0], -s["loc"][1], s["loc"][2]]) / 100
    return m

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

def out(m, bone=None):
    d = {"loc": [round(float(v), 5) for v in m[:3, 3]], "rot": quat(m[:3, :3])}
    if bone: d["bone"] = bone
    return d

lib = json.load(open(f"{M}/_work_att/attlib.json"))
items = {}
for e in lib:
    j = json.load(open(f"{M}/_work_att/att/{e['id']}/{e['id']}.json"))
    items[e["id"]] = {"kind": e["kind"], "sockets": {s["name"].lower(): out(mat(s)) for s in j["sockets"]}}
atts = json.load(open(f"{M}/_work/attachments.json"))
guns = {}
for g in sorted(os.listdir(f"{M}/_work/guns")):
    gj = f"{M}/_work/guns/{g}/{g}.json"
    if not os.path.exists(gj): continue
    rs = {s["name"].lower(): s for s in json.load(open(gj))["sockets"]}
    sock = {n: out(mat(s), s["bone"]) for n, s in rs.items() if n not in ("sticker", "eject", "magazine", "magazine_spare", "trigger")}
    for p in atts.get(g, {"parts": []})["parts"]:
        pj = f"{M}/_work_att/parts/att/{g}__{p['row'].replace(' ', '_')}/{g}__{p['row'].replace(' ', '_')}.json"
        if not os.path.exists(pj): continue
        ms = rs.get(p["socket"].lower()) or rs.get(p["socket"].split("_")[-1].lower())
        if not ms: continue
        base = mat(ms)
        for s in json.load(open(pj))["sockets"]:
            n = s["name"].lower()
            if n in ("ammo", "ammo1", "ammo2", "trigger", "lightdecal") or n in sock and n not in ("aimpoint",): continue
            sock[n] = out(base @ mat(s), ms["bone"])
    guns[g] = sock
FOLDER = {"m4a1": "M4A1", "mglock17": "Glock17", "glock": "Glock17", "akm": "Akm", "deagle": "Deagle", "fal": "FAL", "mossberg590": "Mossberg590", "remington700": "Remington700",
          "revolver": "Revolver", "scar-h": "SCAR-H", "svd": "SVD", "uzi": "UZI", "mp9": "MP9", "vector": "Vector", "sg5-k": "MP5", "benelli": "Benelli", "crossbow": "Crossbow", "vaiga": "Vaiga"}
rows = {}  # weapon row -> gun glb (mirrors viewmodel::gun_model)
for f in glob.glob(os.path.join(HERE, "..", "assets", "weapons", "*.json")):
    w = json.load(open(f)); r = w["row"]
    if r in ("CZ-P-09", "Kobra", "M1911"): rows[r] = r; continue
    fire = (w["anim_set"].get("Fire") or [{}])[0]
    path = (fire.get("Weapon") or fire.get("Character") or {}).get("path", "")
    if "/Animations/" in path and (g := FOLDER.get(path.split("/Animations/")[1].split("/")[0].lower())): rows[r] = g
json.dump({"items": items, "guns": guns, "rows": rows}, open(os.path.join(HERE, "..", "assets", "attachments.json"), "w"), separators=(",", ":"))
print({g: sorted(k for k in s if k in ("optic", "optic_mount", "canted_optic_mount", "muzzle", "underbarrel", "upperbarrel", "siderail")) for g, s in guns.items()})
