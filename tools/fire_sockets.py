"""Bakes the muzzle (WEP.InitFireLocationTarget: BulletSpawnOrigin attached to the 'FireLocation' socket of the barrel/muzzle part)
of every exported gun into assets/fire_sockets.json: {gun: [bone, x, y, z]} in UE cm, relative to that bone of the gun skeleton.
FireLocation comes from the default barrel part's mesh (extracted/json), composed with the receiver socket the part hangs on
(rotation included). Guns without one (receiver-integrated barrels) fall back to a receiver FireLocation socket, else are omitted.
Usage: python -I tools/fire_sockets.py"""
import json, math, os

ROOT = os.path.join(os.path.dirname(__file__), "..")
W = os.path.join(ROOT, "assets-local/models/_work")
X = os.path.join(ROOT, "extracted/json/Content")


def mesh_sockets(asset):
    p = os.path.join(X, asset.split("Content/", 1)[1] + ".json")
    if not os.path.exists(p):
        return {}
    out = {}

    def walk(o):
        if isinstance(o, dict):
            pr = o.get("Properties", o)
            if isinstance(pr, dict) and "SocketName" in pr:
                v = pr.get("RelativeLocation", {})
                out[pr["SocketName"].lower()] = [v.get("x", 0.0), v.get("y", 0.0), v.get("z", 0.0)]
            for v in o.values():
                walk(v)
        elif isinstance(o, list):
            for v in o:
                walk(v)

    walk(json.load(open(p, encoding="utf-8")))
    return out


def rotate(rot, v):
    """UE FRotator (pitch, yaw, roll) degrees applied to v (FRotationMatrix: roll about X, pitch about Y, yaw about Z)."""
    p, y, r = (math.radians(a) for a in rot)
    sp, cp, sy, cy, sr, cr = math.sin(p), math.cos(p), math.sin(y), math.cos(y), math.sin(r), math.cos(r)
    m = [
        [cp * cy, sr * sp * cy - cr * sy, -(cr * sp * cy + sr * sy)],
        [cp * sy, sr * sp * sy + cr * cy, cy * sr - cr * sp * sy],
        [sp, -sr * cp, cr * cp],
    ]
    return [sum(m[i][j] * v[j] for j in range(3)) for i in range(3)]


att = json.load(open(os.path.join(W, "attachments.json")))
out = {}
for gun, v in sorted(att.items()):
    gj = os.path.join(W, "guns", gun, gun + ".json")
    if not os.path.exists(gj):
        continue
    socks = {s["name"].lower(): s for s in json.load(open(gj))["sockets"]}
    for part in v["parts"]:
        fl = mesh_sockets(part["mesh"]).get("firelocation")
        s = socks.get(part["socket"].lower()) or socks.get(part["socket"].split("_")[-1].lower())
        if fl and s:
            r = rotate(s["rot"], fl)
            out[gun] = [s["bone"]] + [round(a + b, 3) for a, b in zip(s["loc"], r)]
    # No FireLocation (MP5): UE logs "'FireLocation' socket not found" and leaves BulletSpawnOrigin where it was; the
    # receiver's Muzzle socket is the closest stand-in. ponytail: real origin then is the WEP root Scene component.
    key = "firelocation" if "firelocation" in socks else "muzzle"
    if gun not in out and key in socks:
        s = socks[key]
        out[gun] = [s["bone"]] + [round(a, 3) for a in s["loc"]]
json.dump(out, open(os.path.join(ROOT, "assets/fire_sockets.json"), "w"), indent=1)
print(json.dumps(out))
