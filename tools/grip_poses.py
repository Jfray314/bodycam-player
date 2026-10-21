"""Grip pose clips (DT_WeaponItems CosmeticEffects Cosmetic.Weapon.Anim.Idle of grip attachments) -> assets/grip_poses.json:
first-key local rotation (xyzw) of the left-hand finger bones, read from the exported arm clips (assets-local/models/arms_anims/*.glb).
ABP_Procedural_Viewmodel's LeftHandPose layer blends this pose onto the left hand while the hand sits on the grip.
python -I tools/grip_poses.py"""
import json, os, struct, glob
import numpy as np
HERE = os.path.dirname(os.path.abspath(__file__))
X = "F:/code/bodycam-player/extracted/json/Content/BodycamCore/ItemsDefinition/Gameplay/DT_WeaponItems.json"
rows = json.load(open(X))["exports"][0]["properties"]["_rows"]
clips = {e.rsplit(".", 1)[-1] for v in rows.values() for k, e in (v.get("GameplayDataDefinition", {}).get("CosmeticEffects") or {}).items() if k == "Cosmetic.Weapon.Anim.Idle"}
FINGERS = [f"{f}_0{i}_l" for f in ("thumb", "index", "middle", "ring", "pinky") for i in (1, 2, 3)]
def quat(R):
    from math import sqrt
    t = np.trace(R)
    if t > 0:
        k = sqrt(t + 1) * 2; q = [(R[2, 1] - R[1, 2]) / k, (R[0, 2] - R[2, 0]) / k, (R[1, 0] - R[0, 1]) / k, k / 4]
    else:
        i = int(np.argmax(np.diag(R))); a, b = (i + 1) % 3, (i + 2) % 3
        k = sqrt(R[i, i] - R[a, a] - R[b, b] + 1) * 2; q = [0, 0, 0, 0]
        q[i] = k / 4; q[a] = (R[a, i] + R[i, a]) / k; q[b] = (R[b, i] + R[i, b]) / k; q[3] = (R[b, a] - R[a, b]) / k
    return [round(float(v), 5) for v in q]

out = {}
for f in glob.glob(os.path.join(HERE, "..", "assets-local", "models", "arms_anims", "*.glb")):
    b = open(f, "rb").read()
    n = struct.unpack("<I", b[12:16])[0]
    j = json.loads(b[20:20 + n])
    binoff = 20 + n + 8
    def acc(i):
        a = j["accessors"][i]; v = j["bufferViews"][a["bufferView"]]
        o = binoff + v.get("byteOffset", 0) + a.get("byteOffset", 0)
        k = {"SCALAR": 1, "VEC3": 3, "VEC4": 4}[a["type"]]
        return struct.unpack_from(f"<{k}f", b, o)
    for an in j.get("animations", []):
        if an["name"] not in clips or an["name"] in out:
            continue
        pose = {}
        local = {i: dict(t=nd.get("translation", [0, 0, 0]), r=nd.get("rotation", [0, 0, 0, 1])) for i, nd in enumerate(j["nodes"])}
        for c in an["channels"]:
            ni = c["target"]["node"]; name = j["nodes"][ni].get("name"); path = c["target"]["path"]
            v = acc(an["samplers"][c["sampler"]]["output"])
            if path in ("rotation", "translation"):
                local[ni][path[0]] = list(v)
            if name in FINGERS and path == "rotation":
                pose[name] = [round(x, 5) for x in v]
        # hand_l relative to ik_hand_gun (the gun's attach joint) in this pose: the hand orientation for holding the grip
        par = {c: i for i, nd in enumerate(j["nodes"]) for c in nd.get("children", [])}
        def glob(i):
            m = np.eye(4)
            while i is not None:
                l = local[i]; x, y, z, w = l["r"]
                R = np.array([[1-2*(y*y+z*z), 2*(x*y-z*w), 2*(x*z+y*w)], [2*(x*y+z*w), 1-2*(x*x+z*z), 2*(y*z-x*w)], [2*(x*z-y*w), 2*(y*z+x*w), 1-2*(x*x+y*y)]])
                t = np.eye(4); t[:3, :3] = R; t[:3, 3] = l["t"]
                m = t @ m; i = par.get(i)
            return m
        idx = {nd.get("name"): i for i, nd in enumerate(j["nodes"])}
        rel = np.linalg.inv(glob(idx["ik_hand_gun"])) @ glob(idx["hand_l"])
        out[an["name"]] = {"fingers": pose, "hand_rot": quat(rel[:3, :3]), "hand_pos": [round(float(v), 5) for v in rel[:3, 3]]}
json.dump(out, open(os.path.join(HERE, "..", "assets", "grip_poses.json"), "w"), indent=1)
print({k: (len(v["fingers"]), v["hand_pos"]) for k, v in out.items()}, "missing:", sorted(clips - set(out)))
