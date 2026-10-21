"""Grip pose clips (DT_WeaponItems CosmeticEffects Cosmetic.Weapon.Anim.Idle of grip attachments) -> assets/grip_poses.json:
first-key local rotation (xyzw) of the left-hand finger bones, read from the exported arm clips (assets-local/models/arms_anims/*.glb).
ABP_Procedural_Viewmodel's LeftHandPose layer blends this pose onto the left hand while the hand sits on the grip.
python -I tools/grip_poses.py"""
import json, os, struct, glob
HERE = os.path.dirname(os.path.abspath(__file__))
X = "F:/code/bodycam-player/extracted/json/Content/BodycamCore/ItemsDefinition/Gameplay/DT_WeaponItems.json"
rows = json.load(open(X))["exports"][0]["properties"]["_rows"]
clips = {e.rsplit(".", 1)[-1] for v in rows.values() for k, e in (v.get("GameplayDataDefinition", {}).get("CosmeticEffects") or {}).items() if k == "Cosmetic.Weapon.Anim.Idle"}
FINGERS = [f"{f}_0{i}_l" for f in ("thumb", "index", "middle", "ring", "pinky") for i in (1, 2, 3)]
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
        for c in an["channels"]:
            name = j["nodes"][c["target"]["node"]].get("name")
            if name in FINGERS and c["target"]["path"] == "rotation":
                pose[name] = [round(v, 5) for v in acc(an["samplers"][c["sampler"]]["output"])]
        out[an["name"]] = pose
json.dump(out, open(os.path.join(HERE, "..", "assets", "grip_poses.json"), "w"), indent=1)
print({k: len(v) for k, v in out.items()}, "missing:", sorted(clips - set(out)))
