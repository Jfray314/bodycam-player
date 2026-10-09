"""Pure-python glb inspector (no Blender needed): prints joints, clips (name, duration), materials and their textures.
python -I tools/blender/inspect_glb.py a.glb [b.glb ...]   |   import inspect_glb; inspect_glb.info(path) -> dict
"""
import json, struct, sys


def info(path):
    b = open(path, "rb").read()
    n = struct.unpack_from("<I", b, 12)[0]
    j = json.loads(b[20:20 + n])
    acc = j["accessors"]
    clips = []
    for a in j.get("animations", []):
        dur = max(acc[s["input"]]["max"][0] for s in a["samplers"])
        clips.append({"name": a["name"], "duration": round(dur, 4)})
    mats = []
    for m in j.get("materials", []):
        p = m.get("pbrMetallicRoughness", {})
        tex = lambda t: j["images"][j["textures"][t["index"]]["source"]].get("name") if t else None
        mats.append({"name": m["name"], "baseColor": tex(p.get("baseColorTexture")), "baseColorFactor": p.get("baseColorFactor"),
                     "orm": tex(p.get("metallicRoughnessTexture")), "normal": tex(m.get("normalTexture")), "occlusion": tex(m.get("occlusionTexture"))})
    joints = sum(len(s["joints"]) for s in j.get("skins", []))
    return {"file": path, "bones": joints, "meshes": len(j.get("meshes", [])), "clips": clips, "materials": mats}


if __name__ == "__main__":
    for p in sys.argv[1:]:
        r = info(p)
        print(r["file"], "bones", r["bones"], "clips", len(r["clips"]))
        for c in r["clips"]:
            print("  clip", c["name"], c["duration"])
        for m in r["materials"]:
            print("  mat", m)
